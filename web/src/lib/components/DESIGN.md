# Marcan UI design guide

Rules for anyone changing the web UI; a component that cannot follow one changes the rule here
first. What a component does is in its own doc comment.

## What the UI may decide

The server decides (`crates/dmarc-assess`, `crates/dmarc-store/src/outcome.rs`); the UI only
words and colours. It never re-derives a cutoff or predicate, so every page that shows the same
record reaches the same conclusion about it.

- A verdict is anything that ranks a row: a tone, a tint, a severity, a warning label. Only
  evidence decides it (counts, auth results, alignment, disposition, published policy, DNS
  findings), never who owns the netblock.
- Sentences that judge (alert events, issues, policy readiness, client reports, the letterhead
  preview) arrive worded from the server's `wording/`; the UI shows them as they are.
- The UI keeps two orders as presentation: `TONE_RANK` (the worst of a card's tones) and
  `SEVERITY_RANK` in `$lib/dns`. Any other order arrives with the data (`outcome_rank`,
  `attention_rank`, the pipeline's `headline`).
- Numbers in help text and the ranges a field checks come from `GET /api/meta` (`meta`) or a
  group's `defaults`, never a literal.

## UI rules

1. **One verdict per card, worst wins.** A card's border and dot show the worst of its rated
   contents; information (forwarded mail, a count) never colours a card. Score points lost
   take the colour of the findings that cost them.
2. **Colour = consequence.** Mail lost, or delivered only because the policy is lax: red.
   Spoofing stopped: blue (`info`, used for nothing else). Forwarded: violet (`forward`).
   Amber (`warn`): worth fixing. All passed: green. Inventory or explanation: neutral.
3. **Blocked by policy is blue everywhere** (charts, tiles, badges, counts).
4. **Fixed status words.** Present: "Published". Absent: "No record" (never "Not published"
   or "Unknown"). Lookup timed out or errored: "Check failed". Empty value: "—". A failed or
   incomplete latest DNS attempt shows the last complete result with an amber "update failed,
   retrying" line; with none, every card reads "Check failed". Partial results are never shown.
5. **Sentence case** for headings, labels, column headers, tabs and buttons. No CSS
   `capitalize` on data.
6. **Monospace only for literal DNS/record/tag text, IPs and selectors** (`InlineCode` inside
   a sentence). Numbers use the sans font with `tabular-nums`.
7. **Numbers and dates go through `$lib/format`**; counts with nouns through `plural` /
   `pluralNoun`, shares through `percent`. Fractions read "x of y". No `toLocaleString` at call
   sites. Typed lists are split by `$lib/text`.
8. **A badge is a row's verdict only; cards have none.** Never label and value in one badge
   ("MTA-STS: Missing"); never show a default (`pct=100%`).
9. **Every tinted chip is `StatusBadge`.** No hand-built `Badge` + `statusSurface`.
10. **One heading style per level:** page `PageHeader`; section h2 `text-sm font-medium`; card
    or drawer h3 `text-sm font-semibold`; small-caps label `DetailHeading`.
11. **No text that repeats its own badge or the number next to it.**
12. **One wording table per server enum** (`Record<Enum, …>`, so the compiler keeps it
    complete; `wording/tables` tests the shared ones), next to its only user, or in `$lib/wording` when shared; never a `switch` over
    an enum in a component. Tone maps live in `$lib/tokens`. No cutoff appears in the UI.
13. **No "Loading…" text and no inline spinner;** skeletons only.
14. **Tables scroll with the page.** No table has a scroll box or fixed height of its own; a
    long list is paged (50 rows on a server list). Where the table fits its column the header
    sticks under the app header; where it does not (a phone) it scrolls sideways.
15. **One outcome legend.** The four outcomes with their counts are always `OutcomeLegend` (a
    chart's footer, or standing alone); pages pass data, never a layout or a legend of their
    own. The one exception is the organizations list: one `OutcomeBar` per row.

## Layout, loading and errors

- The app header holds the trail (`crumbsForPath`); pages never render their own breadcrumb.
  Page actions wrap under the title at 375 px and never widen the page.
- Every load is a `resource()` rendered by `Loaded` (a Settings part by `SettingsPart`); no
  page hand-writes that shell and none uses `{#await}`. Skeletons only before the first answer
  for a key; a refresh, re-sort or next page keeps the content mounted.
- A failed refresh keeps the last data with `RefreshError` above it; a first failure is
  `ErrorState` with a retry. A 404 or invalid id is `NotFoundState` with a way back.
- One exception to rule 13: the domain overview says "Checking live DNS" in words.
- A busy action is an `Action` with `BusyButton` and `FeedbackAlert`; a destructive one is
  confirmed by `ConfirmDialog`, never `window.confirm`.
- Every internal `href` and `goto` goes through `resolve()`. One Export button per page
  (`ExportDialog`), never a separate export link. No `layerchart` import in pages; use
  `$lib/components/charts/`. Import each component by its path; there is no barrel file.

## Settings pages

- One page per topic in four groups (`$lib/settings-nav`); no tabs. One form per page with one
  sticky `SaveBar`; nothing saves on its own. Actions that act at once go after the form.
- A feature's switch is the form's first row and a draft field: a flipped switch is an unsaved
  change saved with the rest (tested per model). Off disables and dims the rows below (one fieldset) and keeps their values; the save
  bar is never dimmed.
- A disabled Save always says why. While the page is unsaved, only the page's `reason` disables
  it (a model's `blocked`, else its `held`: the check that failed), stated beside Save in the
  warn tone, in the server's words; the field it names is marked invalid (`ariaInvalid`, with
  the message under the field where the page has one). The setup's buttons do the same.
- Unsaved edits hold another page visibly in the save bar, never by a silent cancel or a
  dialog; closing the browser tab is the browser's question (`settings-form` tests).
- A stored secret is a locked "Stored" field with Replace (Remove for the operator key); it
  never enters a draft, and a typed one is a change, never compared. Every test and preview
  uses the values on screen and goes to the test recipient set under Sending.
- No folding, except Alerts' Alert kinds. Below `md` the name sits above the control; the
  control cell is a size container (`@md:` / `@sm:` variants, never viewport `sm:`), and
  nothing in Settings scrolls sideways at 375 px.

### Settings copy

- A label is a noun ("Host", "Sender address"); a checkbox says what it does ("Same login as
  the mailbox"); a feature switch says what the feature does ("Send alerts").
- A row hint is at most 12 words: how to fill it in, a default, or a consequence. Never a
  promise or reassurance (those live in `docs/security.md`), a restatement of the label, a
  number the field shows, or a state sentence ("Off: …"). No hint beats a weak one.
- A page header is the title, one sentence of what the page configures, and one "Docs" link.
  A dependency is named under the switch only while it is not met.
- Under a control goes only live state: validation, a consequence computed from the values,
  what a test answered.
- A validation sentence is the server's (its 409 or 422 `error`). The UI keeps its own copy
  only where it holds a request, worded like the server's, and its doc comment says so.

## Colour tokens

Defined in `src/routes/layout.css` (both themes) and exposed as Tailwind colours; the class
helpers are in `$lib/tokens` (`statusSurface`, `statusRow`, `statusFg`, `statusBorderL`,
`statusDot`, `statusText*`, `statusFigure`). Never hardcode `red-*` / `emerald-*`.

| Token            | Means                                                    |
| ---------------- | -------------------------------------------------------- |
| `status-pass`    | Passed, protected, enforced                              |
| `status-fail`    | Mail lost or delivered failures, critical posture        |
| `status-warn`    | Worth fixing (warning findings, softfail, update failed) |
| `status-info`    | Blocked by policy (spoofing stopped); nothing else       |
| `status-forward` | Forwarded mail: failed in transit, receiver trusted it   |
| `status-neutral` | No verdict: inventory, unknown, no data, minor findings  |

- DNS severities map through `SEVERITY_TONE`: `info` and `notice` are neutral, not blue.
- Outcome colours come from `OUTCOMES` in `$lib/wording/outcome`; other chart series cycle
  `--chart-1` … `--chart-5`.
- Never convey pass/fail with colour alone; keep a label or icon.

## Words

- **Delivered failures**: failed DMARC and still delivered, forwarded mail excepted
  (`delivered_failures`). Never "threats". Red.
- **Forwarded**: failed DMARC and delivered because the receiver trusted the forwarder
  (`forwarded_failures`). Not a delivered failure, not in delivered compliance. Violet.
- **Blocked by policy**: failed DMARC and quarantined or rejected (`blocked_messages`). Blue.
- **Failed** (a bare count): delivered failures plus blocked, never forwarded
  (`failed_messages`). A count that takes forwarded mail in names it
  (`dmarc_failed_incl_forwarded`, the `all_failures` filter).
- **DMARC pass**: the passing outcome everywhere (legend, badge, count line).
- **Delivered compliance**: `compliant / (compliant + delivered failures)`. A trend is signed
  with one decimal and `%`, its comparison in the tooltip only. A rate reads "rate (count)": the rate is
  coloured, the count never; an unmeasured rate is "—".
- Checks read "Pass", "Pass for <domain>", "Fail", "None"; never "aligned".
- **Parked**: sends no mail of its own; expects `p=reject`; grey badge. **Ignored**: not the
  user's domain; the action is "Ignore domain", never red. **Organization**: the user's
  grouping, never "company". **Reporter**: the receiver that sent a report (`org_name`), never
  "organization". Missing mailbox login: "Not set up". Accepted mail: "delivered", never
  "reached inboxes".

### Which compliance figure to show

Blocked mail is a win at estate level and a loss at sender level, so the scopes differ on purpose.

- **Estate, domain, report**: delivered compliance. A quarantined or rejected spoof is out of
  the denominator, so a domain blocking an attack is not penalised.
- **One sender**: raw pass rate, coloured by the same 99 / 95 rule. Blocked mail from a
  legitimate sender is lost mail; the delivered figure would show a quarantined ESP as 100%.

A raw figure is fine as inventory; a verdict on a confounded number is not.

## Identity captions

Identity (`identity.*`) is a **neutral caption only**: it never changes a tint, severity, badge
tone, sort order, ranking or advice. The server names every row; the UI shows it through
`SenderName` / `displayName` (`name`, else `operator_domain`, else "Unknown"), marked
unconfirmed when reverse DNS failed forward confirmation. There are no sender categories.

## Rules the tests pin

- Each settings group is written by exactly one Settings page (`settings-ownership.test.ts`).
- A page form saves its unsaved parts in page order, `saveLast` after, and stops at the first
  failure, naming what was saved (`page-form`).
- A disabled Save states its reason in the warn tone and marks the field (`save-bar`,
  `sending-settings-form`, `alerts-settings-form`, `list-editor` component tests).
- The legend shows all four outcomes in order, a zero muted in place, a trend only where the
  server sent one, the share and meaning in the title (`outcome-legend`).
- With no DNS answer every card reads "Check failed" with no points (`dns.pillars`).
- The scope URL omits every default, carries only what the page reads, and opening an
  organization page never narrows the scope (`scope-url`).
- A late answer never lands, and a failed refresh keeps the last data (`resource`).
- The security select words each choice without a port number (`connection`).
