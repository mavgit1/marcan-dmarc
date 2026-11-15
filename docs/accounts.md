# Accounts

An **account** is one login at one mail provider: an address, the credential Marcan logs in with, and how it reaches the mailbox: the provider's IMAP and SMTP servers, or Microsoft Graph for Microsoft 365. Marcan reads DMARC reports from **any number of accounts** and sends its mail (alerts, digests, client reports) **through one of them**, the one chosen under **Settings → Sending**. An account can receive, send or both.

Accounts live under **Settings → Accounts**; the setup's **Accounts** step uses the same form.

<Screenshot name="accounts" alt="Settings, Accounts: one card per account with its address, where its reports land, whether Sending uses it and its last check, Add account, and below them the schedule and the report arrival warnings" />

Each card is one account: its name and address, "Reports sent to … land here" while it receives, "Sends mail" when Sending goes through it, the state of its last check, and a warning while its credential is locked, expiring or expired. A card opens the account's page. Below the cards, one form for every account: the [schedule](#schedule-and-report-arrival) and the report arrival warnings.

## The account form

**Add account** opens with one choice: the provider, **IMAP/SMTP**, **Google** or **Microsoft 365**, nothing picked. The rest of the form appears once one is picked, and the choice can still be changed until the account is saved. A saved account names its provider in a plain line at the top: another provider means other servers and other credentials, so it is another account (add it, then delete the old one).

| Field | Notes |
| :--- | :--- |
| Provider | **IMAP/SMTP** (any provider), **Google** or **Microsoft 365**. The provider fills in the servers (Microsoft 365 has none) and decides the login; the line under each links its section below. Fixed once saved. |
| Name | How Settings, the setup and the logs name it. Empty: the address. |
| Email address | The address reports are sent to, and the sender when the account sends. It is the account's [report address](#the-report-address). An address belongs to one account only. |
| Login | A **password** (username and password) for IMAP/SMTP and Google, the username starting as the address; a **Microsoft login** for Microsoft 365: the tenant and client ID of an app registration in Microsoft Entra and the certificate Marcan generates for it (**Generate**, its thumbprint and expiry, **Download .cer**). |
| Receive reports | On: Marcan reads the account's **Folder** (INBOX by default; the list fills once the login is tested, with how many messages the folder holds). **Reports forwarded from other addresses?** adds the [forwarded addresses](#reports-forwarded-from-other-addresses), one row each. Off: the account only sends. |
| Advanced | A password login's IMAP and SMTP servers (host, security, port), folded into one line: the provider fills them in. It opens by itself when servers have to be entered. Without an SMTP server the account cannot send. A Microsoft 365 account has no Advanced. |
| Test | A receiving account's last check first, as its card says it ("Error, checked 5 minutes ago"), with the reason when it failed (and the server's own answer under **Details**, when it gave one). **Test** logs in with the values on the page, saved or not, and says per step what happened: see [Testing an account](#testing-an-account). |

**Save** tests a changed login, or receiving switched on, first; switching receiving off needs no test, so a mailbox whose login fails can still be stopped. A newly generated certificate is saved without a test, since Microsoft accepts it only once it is uploaded, which comes after the save ("Saved: upload the certificate to the app registration, then test"). **Delete account**, at the end of the page, removes the account with its credentials and its import state; the reports it imported stay. Marcan refuses to delete the account Sending goes through while sending is on (choose another first, or turn sending off).

Security is **TLS** (TLS from the first byte) or **STARTTLS** (a plain connection upgraded to TLS before logging in). Unencrypted IMAP and SMTP are not supported: a server that does not offer STARTTLS is an error, never a fallback. An empty port is the usual one for the security: IMAP 993 with TLS and 143 with STARTTLS, SMTP 465 with TLS and 587 with STARTTLS. SMTP port 25 is refused.

Credentials go only where they were saved for. A changed IMAP or SMTP host, port or username asks for the password again. A Microsoft login's certificate key never leaves Marcan at all: see [Microsoft 365](#microsoft-365). None of them is ever returned by the API or written to the logs, and they are stored encrypted when `MARCAN_SECRET_KEY` is set (see [Secrets at rest](./security#secrets-at-rest)). Without the key, the form says "Not encrypted" under the login, linking there.

Marcan never changes a mailbox: over IMAP it opens the folder read-only and reads messages without marking them as seen, and over Microsoft Graph it only reads (the role `Application Mail.Read` allows nothing else). You can keep reading the same mailbox in a mail client, or clean it up with your own rules.

## Providers

| Provider | IMAP | SMTP | Login |
| :--- | :--- | :--- | :--- |
| [IMAP/SMTP](#imap-smtp) | entered under Advanced | entered under Advanced, or none | password |
| [Google](#google) | `imap.gmail.com`, 993, TLS | `smtp.gmail.com`, 465, TLS | app password |
| [Microsoft 365](#microsoft-365) | none: reads over Microsoft Graph | none: sends over Microsoft Graph | Microsoft login (certificate) |

## IMAP/SMTP {#imap-smtp}

For your own mail server or a hosting provider:

1. **Add account**, Provider **IMAP/SMTP**.
2. Name and Email address, such as `dmarc@reports.example.net`.
3. Username (it starts as the address) and password. Use a dedicated mailbox, or an app password that can be revoked on its own.
4. Under **Advanced**, the IMAP server from your provider's documentation, such as `imap.example.net` with TLS on 993, and, if the account should send, the SMTP submission server, such as `smtp.example.net` with STARTTLS on 587.
5. **Test**, pick the folder, **Save**. The import of the folder starts at once.

## Google Workspace and Gmail {#google}

Provider **Google** fills in Google's servers; the login is an **app password**. It is an IMAP/SMTP account with Google's servers filled in: one whose IMAP server is `imap.gmail.com` shows as Google again.

- IMAP `imap.gmail.com`, 993, TLS; SMTP `smtp.gmail.com`, 465, TLS (or 587 with STARTTLS).
- Username: the full address.
- Password: an app password, created at [myaccount.google.com/apppasswords](https://myaccount.google.com/apppasswords). It needs 2-Step Verification on the Google account, and Google shows the 16 characters once. Changing the account's Google password revokes its app passwords.

Google itself describes app passwords as less secure than its own sign-in and recommends them only for apps that have no other way to sign in, which is Marcan's case. For a work or school account the administrator decides: the app password option is missing when the organization has turned it off, when the account signs in with security keys only or is under Advanced Protection, and IMAP must be allowed in the Google Admin console (**Apps → Google Workspace → Gmail → End User Access**). A dedicated Google account for the reports keeps this apart from anyone's own mailbox.

## Microsoft 365 {#microsoft-365}

A Microsoft 365 mailbox is read and sent from over **Microsoft Graph** with a **Microsoft login**: an app registration in Microsoft Entra that logs in as itself (the OAuth 2.0 client credentials grant) and proves itself with a certificate Marcan generates. No user signs in and no password is involved. The app gets access to the one report mailbox in Exchange Online, with PowerShell, and to nothing else. This is the only setup Marcan supports. It takes an administrator of the tenant once, in this order.

<Screenshot name="account" alt="A new Microsoft 365 account: name, email address, Microsoft login with tenant and client ID, a generated certificate with its thumbprint, expiry and Download .cer, Receive reports with the folder, and Test" />

### What you need {#ms-requirements}

- **A mailbox for the reports.** A shared mailbox is fine and needs no licence; a user mailbox works too.
- **An administrator** who may register apps in Microsoft Entra and manage Exchange Online (an Exchange administrator, a member of the Organization Management role group in Exchange Online).
- **PowerShell with the `ExchangeOnlineManagement` module.** On macOS or Linux, install PowerShell first (`brew install powershell`, or Microsoft's packages) and start it with `pwsh`. Then, in PowerShell:

  ```powershell
  Install-Module ExchangeOnlineManagement
  ```

  It asks whether to install from an untrusted repository (the PowerShell Gallery). Answer `Y`: the default answer is No, which installs nothing.

### 1. Register the app {#register-the-app}

In the [Microsoft Entra admin center](https://entra.microsoft.com), **App registrations → New registration**:

- **Name**: anything you recognise, such as `Marcan`.
- **Supported account types**: **Single tenant only** (in the older form: **Accounts in this organizational directory only**).
- **Redirect URI (optional)**: leave it empty; **Select a platform** can be left as it is. **Register**.

The app's **Overview** shows the two IDs Marcan asks for: **Application (client) ID** and **Directory (tenant) ID**. **Supported account types** there says **My organization only**.

### 2. Add the account in Marcan and upload its certificate {#certificate}

In Marcan, **Settings → Accounts → Add account**, Provider **Microsoft 365**:

1. **Name**, and the mailbox's primary address as **Email address**: Marcan reads and sends as it. When the reports go to an alias of the mailbox, enter that alias under [Reports forwarded from other addresses](#reports-forwarded-from-other-addresses).
2. **Tenant ID** (the Directory (tenant) ID) and **Client ID** (the Application (client) ID).
3. **Generate**: Marcan makes a key pair and a self-signed certificate valid for two years, and shows its thumbprint and expiry.
4. **Save** (in the setup: **Save and start import**). A new certificate is saved without a test: Microsoft accepts it only once it is uploaded.
5. **Download .cer**.

In Entra, on the app registration: **Certificates & secrets → Certificates → Upload certificate**, choose the `.cer`, **Add**. Entra lists it with the same thumbprint Marcan shows. The private key never leaves Marcan: each token request carries an assertion signed with it.

::: warning Add nothing under API permissions
Do not add any permission under the app registration's **API permissions**. A `Mail.Read` or `Mail.Send` application permission granted there applies to every mailbox of the tenant, and it adds to the Exchange Online roles of step 4, so the app could read or send as any mailbox. The roles of step 4 are all Marcan needs. The delegated `User.Read` that Entra puts there by default is harmless; Marcan does not use it.
:::

### 3. Find the enterprise application's Object ID {#object-id}

Exchange Online needs the Object ID of the app's **enterprise application** (its service principal), not the Object ID on the app registration's Overview: the two differ, and the registration's gives `AADServicePrincipalNotFound` in the next step. On the app registration's **Overview**, click the link next to **Managed application in local directory**, and copy **Object ID** there.

![The app registration's Overview in Entra: the link under Managed application in local directory, bottom right, leads to the enterprise application; the Object ID on this page is not the one Exchange Online needs](/screenshots/entra-overview.png)

![The enterprise application's Overview in Entra: Properties with Name, Application ID and the Object ID to copy](/screenshots/entra-enterprise-app.png)

### 4. Give the app the one mailbox in Exchange Online {#exchange-online}

In PowerShell, signed in as the administrator, with your own client ID, Object ID and mailbox address:

```powershell
Connect-ExchangeOnline

# Make the app known to Exchange Online.
New-ServicePrincipal -AppId <client ID> -ObjectId <enterprise application object ID> -DisplayName "Marcan"

# The one mailbox it may use.
New-ManagementScope -Name "Marcan mailbox" -RecipientRestrictionFilter "PrimarySmtpAddress -eq 'dmarc@example.com'"

# Read it.
New-ManagementRoleAssignment -App <client ID> -Role "Application Mail.Read" -CustomResourceScope "Marcan mailbox"

# Send as it: only if the account sends (alerts, client reports).
New-ManagementRoleAssignment -App <client ID> -Role "Application Mail.Send" -CustomResourceScope "Marcan mailbox"
```

To check what Exchange Online has:

```powershell
Test-ServicePrincipalAuthorization -Identity <client ID> -Resource dmarc@example.com | Format-List RoleName,InScope
```

`InScope : True` for each role (`RoleName : Application Mail.Read`, and `Application Mail.Send` if the account sends) means the setup is right and there is nothing more to do; for another mailbox it says `False`. Microsoft caches the permissions, so Marcan's Test can still be refused for 30 minutes to 2 hours after that: wait and test again, change nothing.

### 5. Test {#ms-test}

Back on the account's page in Marcan, **Test**. Exchange Online can take from 30 minutes to about two hours before a new role assignment applies; until then the test says the app may not read the mailbox (refused although the check above says `True`: wait). The test proves the token and reading the mailbox; pick the folder once it lists them, and **Save**. Sending is proven by **Send a test** under **Settings → Sending**.

### Notes {#ms-notes}

- **Network**: Marcan needs outbound HTTPS to `login.microsoftonline.com` (tokens) and `graph.microsoft.com` (the mailbox). See [Network access](./deployment#network-access).
- **Worldwide cloud only**: national clouds (US Government, China operated by 21Vianet) are not supported.
- **The certificate** is valid for two years. From 30 days before it expires, the account's card warns and the alert [Microsoft certificate expiring](./alerts#microsoft-certificates-expiring) is raised. To renew: **Generate new**, **Save**, **Download .cer**, upload it in Entra, **Test**, then delete the old certificate in Entra. Saved, the new certificate replaces the old one at once; to renew without a gap, download it before saving (the button is there as soon as it is generated), upload it, then save.
- **Tokens** are kept in memory only, renewed 5 minutes before they expire (a mailbox pass starts with at least 20 minutes left), fetched again after Graph refused one or the account was saved, and sent to `graph.microsoft.com` only.
- **Sent mail** is saved in the mailbox's Sent Items.
- **A recipient the server refuses** comes back as a bounce in the mailbox, not as an error in Marcan: Microsoft accepts the message before it reaches anyone.
- **Size**: Microsoft Graph takes a request of up to 4 MB, the message in base64, so a message of about 3 MB at most. A larger one is not sent.

### Remove the access {#ms-remove}

1. In Marcan, delete the account (or change its login): its private key goes with it.
2. In Entra, **App registrations**, delete the app. This ends the access: nothing can log in as the app any more. Entra keeps a deleted registration under **Deleted applications** for 30 days, from where it can be restored or deleted for good.
3. Optionally, in PowerShell, look for what Exchange Online still holds and remove it. Each of the first two commands prints nothing when there is nothing left:

   ```powershell
   Connect-ExchangeOnline

   # The app's role assignments, and the app as Exchange Online knows it.
   Get-ManagementRoleAssignment -RoleAssigneeType ServicePrincipal | Format-Table Name,Role,RoleAssigneeName
   Get-ServicePrincipal | Format-Table DisplayName,AppId

   Remove-ManagementRoleAssignment -Identity "<Name from the first list>"
   Remove-ServicePrincipal -Identity <client ID>

   # The scope, once no assignment uses it.
   Remove-ManagementScope "Marcan mailbox"
   ```

   A scope that is left behind gives nobody anything: it only names the mailbox.

To take away sending only and keep reading, remove the one assignment with the role `Application Mail.Send`.

## Testing an account

**Test** logs in with the values on the page, saved or not, and shows one line per step. It sends no message and stores nothing. A failed line says why in plain words, with the server's own answer under **Details**; when the fix is a setup step of a Microsoft login (the certificate, the IDs, an Exchange Online role), the line ends with a **(docs)** link to that step of this guide, as does a failed **Send a test**.

With a password login:

1. **IMAP** (when an IMAP server is set): connects, starts TLS, logs in, lists the folders and opens the chosen one read-only, then says how many messages it holds.
2. **SMTP** (when an SMTP server is set): connects, starts TLS, logs in, and stops before any message. Port 25 is refused before connecting.

A failure names the step: the host does not resolve, the server cannot be reached, the TLS connection or STARTTLS failed, the login was rejected ("Check the username and password; many providers require an app password"), or the folder does not exist.

With a Microsoft login, the email address must be set first ("Enter the mailbox's email address: Marcan reads and sends as it"). Then:

1. **Token**: a fresh token from Microsoft for the app registration ("Microsoft issued a token for the app registration."). Without one, the next step does not run. The token names no permission: the app's access to the mailbox comes from its role assignments in Exchange Online, and Microsoft Graph says when one is missing.
2. **Mailbox**: the mailbox read over Microsoft Graph as the address ("Read dmarc@example.com over Microsoft Graph."), its folders, and the chosen one with its message count.

There is no sending line: Microsoft Graph has no way to check sending without sending a message, and the test sends none. **Send a test** under **Settings → Sending** proves it.

What a refusal means (the line starts with the sentence quoted here; Details has Microsoft's own answer):

| Line | Microsoft's answer | What to change |
| :--- | :--- | :--- |
| Token: "Microsoft does not know this tenant ID." | AADSTS90002, AADSTS900023 | Copy the **Directory (tenant) ID** from the app registration's Overview. |
| Token: "The tenant has no app registration with this client ID." | AADSTS700016 | Copy the **Application (client) ID** from the Overview; the app must be registered in that tenant. |
| Token: "Microsoft refused the certificate." | AADSTS700027, AADSTS700024, AADSTS700023, AADSTS7000274 | Upload the `.cer` Marcan generated under Certificates & secrets, Certificates (the thumbprints must match), then test again. |
| Token: "Microsoft refused the token request." | Another AADSTS code | Details has Microsoft's code and description. |
| Token: "Could not reach the Microsoft login service (login.microsoftonline.com)." | None | Marcan needs HTTPS to `login.microsoftonline.com` (see [Network access](./deployment#network-access)). |
| Token: "The stored certificate key cannot be used." | None | Generate a new certificate, save, and upload it. |
| Mailbox: "The app registration may not read …" | `403` | Give the app the role `Application Mail.Read` scoped to this mailbox ([step 4](#exchange-online)). A new assignment can take 30 minutes to 2 hours to apply. |
| Mailbox: "… is not a mailbox in this Microsoft 365 tenant." | `404`, `ErrorInvalidUser`, `MailboxNotEnabledForRESTAPI`, `MailboxNotHostedInExchangeOnline` | Enter the address of a mailbox in the app registration's tenant. |
| Mailbox: "… has no folder …" | None | Choose one of the folders the test lists, or INBOX. |
| Mailbox: "Microsoft Graph refused the app registration's token for …, a new one too." | `401` | Check that the tenant ID is the mailbox's tenant. |
| Mailbox: "Could not reach Microsoft Graph (graph.microsoft.com)." | None | Marcan needs HTTPS to `graph.microsoft.com`. |

## The report address

The account's **Email address** is its report address: while **Receive reports** is on, reports sent to it land in the account's folder, and a domain's DMARC record names it in `rua=`:

```txt
_dmarc.example.com.  TXT  "v=DMARC1; p=none; rua=mailto:dmarc@reports.example.net"
```

The setup's **Report addresses** step and the [Guide](./dashboard#pages) list every address reports reach Marcan at, with the address each new DMARC record names: the first receiving account's, in the order of the list. To change an address, change it on its account.

### Reports forwarded from other addresses

Some domains must name an address of their own, such as `dmarc@example.com`, that forwards to one of Marcan's accounts; a service provider often has one such address per client. Marcan cannot see where an address forwards to, so tell it: on the account the mail is forwarded to, **Receive reports → Reports forwarded from other addresses?**, and enter the address; **Add another address** adds a row for the next one, and the cross beside a row removes it. An account takes any number of them; each address belongs to one account only and is never an account's own address. A domain whose record names one of them then counts as set up, as if it named the account's own address.

Until it is entered, a domain whose record names only its forwarding address reads **Report address missing**, unless its reports already arrive.

An upgrade from 0.3 with Report delivery set to forwarded enters them for you: every address the domains' DMARC records named at the last DNS check before the upgrade, other than an account's own address, goes onto the first account (the converted mailbox).

### Authorization of the address domain {#authorization}

A receiver sends a domain's reports to an address on **another organizational domain** only if that address's domain agrees (RFC 9990 §4). Here `example.com` sends its reports to `reports.example.net`, so `reports.example.net` must publish, once for every domain:

```txt
*._report._dmarc.reports.example.net.  TXT  "v=DMARC1"
```

or one record per domain, such as `example.com._report._dmarc.reports.example.net` with the same value. Without either, receivers drop the reports silently. Nothing is needed when the address is under the domain's own organizational domain, such as `dmarc@example.com` or `dmarc@reports.example.com` for `example.com`.

Marcan checks this per **address domain**: the domain of every account's address and of every forwarded address that a monitored domain on another organizational domain names. The setup's Report addresses step shows the exact record for each, with the real addresses, and its state:

| State | Meaning |
| :--- | :--- |
| **Authorized** | The wildcard record is published: every domain may send reports there. |
| **Authorized per domain** | Every domain that needs it has its own record. The wildcard would also cover domains added later. |
| **No wildcard** | Some domain still lacks its authorization; the step shows the wildcard to publish. |
| **Check failed** | The lookup failed; **Re-check** tries again. |

A forwarding address on another organizational domain, such as `dmarc@forwarder.example.net` in `example.com`'s `rua=`, needs the same authorization from `forwarder.example.net`.

### Each domain's status

The setup's **DMARC records** step and the Guide judge each domain from its live DMARC record:

| Status | When |
| :--- | :--- |
| **Reporting** | Its reports arrive, whatever its record names. |
| **Waiting for first report** | The record names a report address of Marcan (an account's or a forwarded one) whose domain accepts the domain's reports or needs to accept nothing; no report has arrived yet. Receivers send about once a day. |
| **Waiting for reports** | As above, but reports arrived before and then stopped. |
| **Report address missing** | A record is published, but its `rua=` names none of Marcan's addresses. Marcan shows the record with the address added. |
| **Authorization missing** | The record names Marcan's addresses, but the domain of every one of them refuses this domain's reports. |
| **Check failed** | The DMARC record, or the authorization, could not be looked up. |
| **No record** | No DMARC record is published. |

A domain's own page shows the same status while no report arrives, read from the domain's stored DNS check, the one its policy badge and DNS checks show; its **Re-check DNS** updates all three.

## Several accounts

Add one account per mailbox your domains' reports go to, for example one per client tenant. Each receiving account has its own folder, import progress and skipped messages; a report that arrives in two of them is stored once.

- **One pass at a time.** Marcan reads one account at a time (the database has one writer). Each account is due its check interval after its last pass ended; of the due ones, the one whose turn is longest ago goes first. A long first import gives way after 2 minutes when another account is due, and resumes where it stopped on its next turn.
- **A failing account backs off.** When a pass fails without importing anything (the server cannot be reached, the login or the Microsoft token is refused), the account is tried again after 30 seconds, then 1, 2, 5 and 15 minutes, then every hour (or at its check interval, when that is longer). A pass that works, saving the account, **Check now**, a re-import and **Test** start over from 30 seconds. A pass that fails after importing some messages is tried again after 30 seconds.
- **Check now** (in the header's status, and on the setup's Import step) reads every receiving account at once, one after the other.
- **The header** shows the accounts together: the worst state and the oldest check.
- **A failing account** reads **Error** on its card and beside **Accounts** in the Settings list; its own page shows the reason under **Test**.
- **Every account counts.** While any receiving account has gone 3 days without a successful read, the [client reports](./client-reports#when-it-is-sent) wait and the [Account not read](./alerts#accounts-not-read) alert is raised. A new account counts from when it was added, so one being set up holds the reports only once it has failed for 3 days; switch **Receive reports** off for an account you are not using.
- **Sending** goes through one account, chosen under **Settings → Sending**; the others only receive, or wait to be chosen.

Changing an account's IMAP server, port, username or folder (a Microsoft 365 account's tenant, address or folder) starts its import again from the first message of the folder; reports already stored are skipped.

## Schedule and report arrival

Below the cards on **Settings → Accounts**, for every account:

| Field | Notes |
| :--- | :--- |
| Checks | Under **Schedule**: how often each account is checked for new mail, in whole minutes. Every 5 minutes by default, at least every minute. |
| Report warning | Under **Report arrival**: how many hours without any new report, across all accounts, before the status in the header warns (and goes red after twice as long). 4, 12, 24, 48 or 72 hours; 24 by default. |
| Domain warning | Under **Report arrival**: how many days without a report before a domain that had reports is flagged with the **reports stopped** issue, which is also an [alert](./alerts). 2, 3, 5, 7, 10, 14 or 30 days; 14 by default. A parked domain is not flagged. |

How long reports are kept is set apart, under **Settings → Data → Retention** (see [Backup](./backup#retention)).

## Locked accounts

An account is **locked** when its stored credentials are encrypted and cannot be read: `MARCAN_SECRET_KEY` is not set, a different key is set than the one they were encrypted with, or the stored value is damaged (the key it was encrypted with does not open it). A locked account neither receives nor sends: Marcan skips it, a send through it is refused, its card says why, and the log names it at start and at each pass or send that fails. After 3 days it raises the [Account not read](./alerts#accounts-not-read) alert, and the [client reports](./client-reports#when-it-is-sent) wait.

There are two ways out: set the original `MARCAN_SECRET_KEY` again and restart (for a damaged value: restore the database from a backup), or enter the account's credentials again (the password, or a new certificate uploaded to the app registration). They are then stored with the current key, or unencrypted without one. See [Secrets at rest](./security#secrets-at-rest).

## Sync and re-import

- **Check now** (in the header's status and in the setup's Import step) reads every receiving account immediately.
- **Re-import all accounts** (**Settings → Data**, Actions) reads every message in every receiving account's folder again and adds the reports that are missing. Reports already stored are skipped. Use it after taking a domain off the ignore list, or when you have moved old reports into a folder.

## When a message cannot be imported

A message is **quarantined**, recorded and skipped, so it never stalls the import, when it is not a valid report. For example: its domain is not a valid hostname, a record count is below 0 or above 1,000,000,000, the report period ends before it begins or begins more than a day in the future, the attachment is larger than 25 MB, or the report unpacks to more than 50 MB (the reports in one message together, too).

A message that fails for another reason (the server returns nothing for it, the download times out, the database rejects it) is retried on the next passes and quarantined after 5 attempts. The header's status counts skipped reports.

## Sending

Marcan sends mail through one account, chosen under **Settings → Sending**. It is **off by default**. The switch at the top of the page, **Sending**, is saved with the rest of the page: while it is off, the fields below it are greyed out and keep their values, and Marcan sends no mail at all. Switched on, **Save** needs an account ("Choose an account to send through under Settings → Sending.") whose login is complete and readable. Marcan uses it for [alert emails and the digest](./alerts) and for [client reports](./client-reports), each on its own page after it.

<Screenshot name="sending" alt="Settings, Sending: the switch on, the account mail goes through, the account's address it sends as with its domain's DMARC policy and SPF ending, Reply-To, the test recipient and Send a test" />

| Field | Notes |
| :--- | :--- |
| Account | The accounts that can send: those with an SMTP server ("Reports (smtp.example.net)"), and those with a Microsoft login, which send over Microsoft Graph ("Reports (Microsoft Graph)"). The server and the login are the account's, set on its page. |
| Sender address | The From address: always the chosen account's **Email address** ("Sending as dmarc@example.com"); an account without one cannot send. Marcan never sends from another address. |
| Reply-To | Where replies go. Empty: the sender address. |
| Test recipient | Where test emails and previews go: this page's **Send a test**, the test alert under **Alerts** and the client report previews. Empty: the sender address. |

Marcan hands its mail to your provider and nothing more: to the account's SMTP server, always logged in, or for a Microsoft login to Microsoft Graph as the account's mailbox (a copy then stays in its Sent Items). It never delivers mail to the recipients' servers itself, and it never signs mail with DKIM. Whether the mail is trusted is up to your provider and your DNS.

**Send through your report mailbox's provider, so SPF and DKIM are its job.** The provider then signs the mail with DKIM and its servers are already in your SPF record, so the sender's domain passes DMARC without any work. A separate SMTP account (a transactional mail service, another provider) needs its own SPF and DKIM set up for the sender's domain first. Once an account is chosen, the page shows the DMARC policy and SPF `all` of its address's domain from Marcan's DNS check, so a sender without SPF stands out at once.

**To send from another address**, add an account for it: on IMAP/SMTP or Google, the same username and password and SMTP server, that address as its **Email address** and **Receive reports** off; then choose it here. The provider must let the login send as that address (an alias, or Send As in Exchange Online). A Microsoft 365 account sends as its own mailbox only.

**Send a test** sends a short message ("Marcan can send mail from this address.") with the values on the page, saved or not, while the switch is on, to the test recipient on the page (the account's address when it is empty); the line under the button says where it goes. When it fails, it says which step failed and why: the host does not resolve, the server cannot be reached, the TLS connection or STARTTLS failed, the login was rejected, the server refused to send from this account (its login may not send as the account's address), or the server refused the message (with its reply). With a Microsoft login it is Microsoft Graph's answer instead: the app may not send as the mailbox (the role `Application Mail.Send` scoped to the mailbox, [step 4](#exchange-online); a new assignment can take 30 minutes to 2 hours to apply), the address is no mailbox in the tenant, the token was refused, Graph is limiting the requests just now, or the message is larger than Graph takes. Microsoft accepts the message before it reaches anyone, so a recipient's server refusing it shows as a bounce in the account's mailbox.

With sending on, anyone who can open Marcan can make it send mail as the sender. See [Security](./security#sending-mail).
