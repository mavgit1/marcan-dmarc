//! The account form says the server's refusals before a save, in copies
//! (`MS_TEXT`, `ACCOUNT_TEXT` in `web/.../account-draft.svelte.ts`): each
//! copy must stay a string the server says, word for word. Read from the
//! sources: the wording modules.

use std::path::Path;

const FORM: &str = "../../web/src/lib/components/settings/account-draft.svelte.ts";
const SERVER: [&str; 2] = ["src/wording/microsoft.rs", "src/wording/accounts.rs"];

fn read(path: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(path);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The string values of `export const {name} = { ... } as const;` in `ts`.
fn ts_strings(ts: &str, name: &str) -> Vec<String> {
    let start = ts
        .find(&format!("export const {name} = {{"))
        .unwrap_or_else(|| panic!("{name} not found in the form"));
    let block = &ts[start..];
    let block = &block[..block.find("} as const;").expect("end of the object")];
    let mut found = Vec::new();
    let mut chars = block.chars();
    while let Some(c) = chars.next() {
        if !matches!(c, '\'' | '"' | '`') {
            continue;
        }
        let mut value = String::new();
        loop {
            match chars.next().expect("an unclosed string") {
                '\\' => value.push(chars.next().expect("an escape")),
                d if d == c => break,
                ch => value.push(ch),
            }
        }
        found.push(value);
    }
    found
}

/// `text` as a Rust string literal (quotes included).
fn rust_literal(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

#[test]
fn the_account_forms_sentences_are_the_servers() {
    let form = read(FORM);
    let server: String = SERVER.iter().map(|p| read(p)).collect();
    let mut checked = 0;
    for table in ["MS_TEXT", "ACCOUNT_TEXT"] {
        let sentences = ts_strings(&form, table);
        assert!(!sentences.is_empty(), "{table} has no sentences");
        for sentence in sentences {
            assert!(
                server.contains(&rust_literal(&sentence)),
                "{table} says a sentence the server does not: {sentence}"
            );
            checked += 1;
        }
    }
    assert!(checked >= 12, "only {checked} sentences found");
}
