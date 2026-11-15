//! A mailbox's folders over Graph: the configured path looked up to its
//! folder ([`Client::resolve`]), and the connection test's listing of every
//! folder as a path ([`probe_graph`]).
//!
//! A path is display names joined with `/`, compared without case; a first
//! segment `INBOX` (any case) is Graph's well-known `inbox`, whatever the
//! mailbox's language calls it, and the listing names that folder `INBOX`.

use super::client::{Client, Fault, Level};
use super::{GraphConfig, GraphProblem};
use crate::imap::config::DEFAULT_FOLDER;
use crate::imap::connect::{sort_folders, ConnectError, FolderStatus, ImapFolder, MailboxProbe};
use serde::Deserialize;
use std::collections::VecDeque;
use std::time::Duration;
use tracing::info;

/// The properties read of a folder.
const SELECT: &str = "$select=id,displayName,childFolderCount,totalItemCount";

/// Folders per page asked for.
const PAGE: u32 = 100;

/// The most pages of one folder list followed.
const MAX_PAGES: usize = 50;

/// The most folders the connection test lists.
pub(crate) const MAX_FOLDERS: usize = 1_000;

/// How deep the connection test lists nested folders (top level is 1).
pub(crate) const MAX_DEPTH: usize = 10;

/// Upper bound for each request of the lookup and the listing.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Upper bound for one complete [`probe_graph`].
const PROBE_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct Folder {
    pub id: String,
    #[serde(rename = "displayName", default)]
    pub display_name: String,
    #[serde(rename = "childFolderCount", default)]
    pub child_folder_count: u64,
    #[serde(rename = "totalItemCount", default)]
    pub total_item_count: u64,
}

#[derive(Deserialize)]
struct FolderPage {
    #[serde(default)]
    value: Vec<Folder>,
    #[serde(rename = "@odata.nextLink")]
    next_link: Option<String>,
}

/// The segments of a configured folder path; empty means [`DEFAULT_FOLDER`].
pub(crate) fn segments(path: &str) -> Vec<&str> {
    let parts: Vec<&str> = path
        .split('/')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if parts.is_empty() {
        vec![DEFAULT_FOLDER]
    } else {
        parts
    }
}

impl Client {
    /// The well-known inbox.
    pub(crate) async fn inbox(&mut self) -> Result<Folder, Fault> {
        let url = self.api().url(
            &["users", &self.mailbox.clone(), "mailFolders", "inbox"],
            Some(SELECT),
        );
        match self
            .get_json(&url, "reading the inbox", REQUEST_TIMEOUT, Level::Mailbox)
            .await?
        {
            Ok(folder) => Ok(folder),
            Err(error) => Err(self.refusal(&error, "reading the inbox", Level::Mailbox)),
        }
    }

    /// The folders at the top of the mailbox (`parent` `None`) or under
    /// folder `parent`, every page, at most `budget` of them (counted down).
    pub(crate) async fn folders_under(
        &mut self,
        parent: Option<&str>,
        budget: &mut usize,
    ) -> Result<Vec<Folder>, Fault> {
        let mailbox = self.mailbox.clone();
        let query = format!("{SELECT}&$top={PAGE}");
        let (url, what, level) = match parent {
            None => (
                self.api()
                    .url(&["users", &mailbox, "mailFolders"], Some(&query)),
                "listing the mailbox's folders",
                Level::Mailbox,
            ),
            Some(id) => (
                self.api().url(
                    &["users", &mailbox, "mailFolders", id, "childFolders"],
                    Some(&query),
                ),
                "listing a folder's subfolders",
                Level::Item,
            ),
        };
        let mut folders = Vec::new();
        let mut next = Some(url);
        let mut pages = 0;
        while let Some(url) = next.take() {
            let page: FolderPage = match self.get_json(&url, what, REQUEST_TIMEOUT, level).await? {
                Ok(page) => page,
                // The parent went away meanwhile: it has no children now.
                Err(_) => return Ok(folders),
            };
            for folder in page.value {
                if *budget == 0 {
                    return Ok(folders);
                }
                *budget -= 1;
                folders.push(folder);
            }
            pages += 1;
            if let Some(link) = page.next_link {
                let url = reqwest::Url::parse(&link)
                    .ok()
                    .filter(|u| self.api().owns(u));
                let Some(url) = url else {
                    return Err(self.foreign(what, &link));
                };
                if pages < MAX_PAGES {
                    next = Some(url);
                }
            }
        }
        Ok(folders)
    }

    /// The folder at `path` (see the module), `None` when there is none.
    pub(crate) async fn resolve(&mut self, path: &str) -> Result<Option<Folder>, Fault> {
        let mut parts = segments(path).into_iter();
        let first = parts.next().unwrap_or(DEFAULT_FOLDER);
        let mut budget = usize::MAX;
        let mut current = if first.eq_ignore_ascii_case(DEFAULT_FOLDER) {
            self.inbox().await?
        } else {
            let top = self.folders_under(None, &mut budget).await?;
            match find(top, first) {
                Some(folder) => folder,
                None => return Ok(None),
            }
        };
        for name in parts {
            if current.child_folder_count == 0 {
                return Ok(None);
            }
            let children = self.folders_under(Some(&current.id), &mut budget).await?;
            match find(children, name) {
                Some(folder) => current = folder,
                None => return Ok(None),
            }
        }
        Ok(Some(current))
    }

    /// The fault of a folder path that names no folder.
    pub(crate) fn no_folder(&self, path: &str) -> Fault {
        self.fault(
            &GraphProblem::NoFolder {
                mailbox: &self.mailbox,
                folder: path,
            },
            "",
        )
    }
}

/// The folder in `folders` called `name`, case aside.
fn find(folders: Vec<Folder>, name: &str) -> Option<Folder> {
    let lower = name.to_lowercase();
    folders
        .into_iter()
        .find(|f| f.display_name.to_lowercase() == lower)
}

/// The connection test over Graph: every folder of `cfg`'s mailbox as a
/// path (`INBOX/DMARC`; at most [`MAX_FOLDERS`], [`MAX_DEPTH`] deep, sorted
/// like IMAP's), and whether `cfg.folder` exists with its message count.
/// A missing folder is reported in [`FolderStatus::exists`], not as an
/// error; the errors are the pass's, as a [`ConnectError`].
pub async fn probe_graph(cfg: &GraphConfig) -> Result<MailboxProbe, ConnectError> {
    info!(host = %cfg.api.host(), folder = %cfg.folder, "reading the mailbox over Graph (probe)");
    let mut client = Client::new(cfg);
    match tokio::time::timeout(PROBE_TIMEOUT, probe(&mut client, &cfg.folder)).await {
        Ok(result) => result.map_err(Fault::into_connect),
        Err(_) => Err(ConnectError {
            stage: crate::imap::connect::ConnectStage::Other,
            message: format!("{} did not respond in time.", cfg.api.host()),
            detail: format!("timed out after {} seconds", PROBE_TIMEOUT.as_secs()),
            docs: None,
        }),
    }
}

async fn probe(client: &mut Client, folder: &str) -> Result<MailboxProbe, Fault> {
    let inbox = client.inbox().await?;
    let mut budget = MAX_FOLDERS;
    let mut found = Vec::new();
    let mut queue: VecDeque<(Folder, String, usize)> = VecDeque::new();
    for top in client.folders_under(None, &mut budget).await? {
        let path = if top.id == inbox.id {
            DEFAULT_FOLDER.to_owned()
        } else {
            top.display_name.clone()
        };
        queue.push_back((top, path, 1));
    }
    while let Some((folder, path, depth)) = queue.pop_front() {
        if folder.child_folder_count > 0 && depth < MAX_DEPTH && budget > 0 {
            for child in client.folders_under(Some(&folder.id), &mut budget).await? {
                let child_path = format!("{path}/{}", child.display_name);
                queue.push_back((child, child_path, depth + 1));
            }
        }
        found.push(ImapFolder {
            name: path,
            delimiter: Some("/".to_owned()),
        });
    }
    let selected = client.resolve(folder).await?;
    Ok(MailboxProbe {
        folders: sort_folders(found),
        selected: FolderStatus {
            name: folder.to_owned(),
            exists: selected.is_some(),
            messages: selected.map(|f| u32::try_from(f.total_item_count).unwrap_or(u32::MAX)),
        },
    })
}
