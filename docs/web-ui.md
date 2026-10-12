> 🇹🇷 Türkçe için tıklayın: [web-ui.tr.md](./web-ui.tr.md)

# QuestDrop — Web UI Design

Date: 2026-10-12 · Scope: [plan.md](./plan.md) phase 5 · API: [openapi.yaml](./openapi.yaml)

This document describes what the QuestDrop web UI should be. Whoever builds the UI can treat it as the contract. Every screen maps to the API as it exists today; what the API still lacks is listed in §10.

> **Status (2026-10-12): implemented.** Code: `src/web.rs`, `templates/`, `static/`, `i18n/`. Open questions in §13 were decided as recommended: drops need login (§5.2), voice option A (§6), English + Turkish (§9); external voice links stay allowed with an "external audio" label. Deviations: CSP `media-src` also allows `blob:` so the recorder can preview a take before upload; the desktop nav switches to the bottom tab bar below 960 px (not 768 px) because four labels plus the handle do not fit in between.

---

## 1. Who is it for, how should it feel?

The target user is ashamed of their abandoned projects and gets distracted easily (has ADHD or feels that way). Every UI decision follows from that:

| Principle | In the UI |
|---|---|
| **No shame, a game** | Not "you quit" but "you dropped the quest into the pool". No closed projects, only "it continues in someone else's hands". Calm, guiding messages instead of red error screens. |
| **30-second rule** | The drop form can be filled in 30 seconds. Required: title, one-liner, link, voice, time, energy. Skills are optional. |
| **One screen, one main action** | One big primary button per page. Secondary actions are small and in the background. |
| **Few choices** | The match screen shows 4 cards (3 + 1 surprise), not an endless list. No choice paralysis. |
| **Novelty dopamine** | The surprise card looks different. The swap moment gets a small celebration (light animation that respects `prefers-reduced-motion`). |
| **Start right away** | After a swap the user lands directly on the "first 2 minutes" task. No extra navigation needed. |
| **Calm** | No notification storms, badges, points or counters (the design doc leaves them out on purpose). |

The success measure is the design doc's: **strangers trading in the open pool**. The UI makes that possible by letting a first-time visitor drop their first quest within a few minutes.

---

## 2. Technical approach

**Recommended: server-rendered in the same Rust binary (Askama + HTMX).** The stack doc already locks this in.

- Pages are rendered on the server with Askama templates. HTMX is used only for partial updates: "load more", inline form errors, swap confirmation.
- **Every form works without JavaScript.** Without HTMX it is a normal form POST plus a page reload; HTMX only speeds it up. The single exception is in-browser voice recording (§6), which has a file-upload fallback.
- UI routes (`/`, `/drop`, `/match`, …) reach the database directly through `db::` functions; they do not call their own REST API over HTTP. REST `/v1/` stays for the bot and external clients. Validation (`validate.rs`), rules and transactions stay in one place.
- Same origin, so the session cookie (`qd_session`, `SameSite=Lax`) just works. **No CORS needed.**
- No build step. No Node, npm or bundler. `static/` holds one CSS file, a pinned copy of HTMX (`htmx.min.js`, about 15 KB) and a small JS file for voice recording.
- Target: every page under 50 KB (excluding audio), usable within 2 seconds on slow 3G.

**Alternative (not recommended):** a separate SPA (React/Svelte, etc.) on another origin. That needs CORS with credentials, CSRF handling and cookie-domain setup (see §10, item 4). It also breaks the stack doc's "no Node" decision.

---

## 3. Site map

| URL | Page | Login | Main action |
|---|---|---|---|
| `/` | Pool | – | "Drop a quest" |
| `/login` | Email login | – | "Send link" |
| `/login?token=…` | Finish login | – | "Log in" |
| `/drop` | Drop | ✔ (§5.2) | "Into the pool" |
| `/match` | 4 quests for you | ✔ | "Take this" |
| `/take/{offer_id}` | Swap confirmation | ✔ | "Swap" |
| `/revive` | Quests I took | ✔ | "Start the first 2 minutes" |
| `/me` | Profile: skills, my open quests, log out | ✔ | "Save" |
| `/q/{project_id}` | Single quest (shareable) | – | "Take this" |

A visitor without a session who opens a login-only page is redirected to `/login?next=/match`, and returns to `next` after login. `next` must be a local path; no open redirects.

**Top menu** (every page; a bottom tab bar on mobile): Pool · Drop · Match · My quests. On the right: `@handle` when logged in, otherwise "Log in".

---

## 4. Main flows

### 4.1 First visit → first drop
1. `/` shows the pool, with a one-line pitch and a "Drop a quest" button on top.
2. "Drop a quest" → without a session, `/login?next=/drop`: email + optional handle → "Send link".
3. Emailed link → `/login?token=…` → "Log in" → `/drop`. Handle and email come from the session and are not shown in the form.
4. Title, one-liner, link, voice, skills, time, energy → "Into the pool".
5. Confirmation: "Your quest is in the pool." + "See quests for me" (`/match`).

Why is dropping the first action? Swaps are two-way: taking requires giving one of your own open quests (design §5.4). Someone who never dropped cannot take.

### 4.2 Login
1. `/login`: email (+ optional handle on first visit). "Send link" → the equivalent of `POST /v1/auth/request`.
2. The screen says "Check your inbox, the link is valid for 15 minutes." Resending to the same email unlocks after 60 seconds.
3. The link opens `/login?token=…`.
   - **The token must not be consumed on GET.** Mail clients' link previewers would burn it, so the page first shows a "Log in" button. The button POSTs the token; on success the cookie is set and the user is redirected to `next` or `/match`.
4. Invalid or expired token: "This link has expired. Send a new one?" plus the email field.

### 4.3 Match → take → revive (core loop)
1. `/match`: 4 cards. If the user has no skills, a banner says "Add your skills for better matches → /me" (without skills the matcher ranks purely at random).
2. "Take this" → `/take/{offer_id}`: the quest to take on top, the user's **open quests** below (radio list). If there is only one, it is preselected.
3. "Swap" → swap (with Idempotency-Key, §7.3) → celebration → straight to `/revive#{match_id}`.
4. `/revive`: the taken quest card with voice player, link and the 4-step "first 2 minutes" task (checkboxes kept only in the browser). Optional 2-minute countdown.

If the user has no open quest, `/take` says "Drop a quest first" instead of offering a swap, and links to `/drop?next=/take/{offer_id}`.

---

## 5. Screens

Think through the states of every screen in the same order: **filled · empty · loading · error · no database (503)**.

### 5.1 Pool (`/`)
- **Header:** "Tired of it? Drop it. Pick up someone else's half-finished quest." + "Drop a quest" (primary) + "Quests for me" (secondary, `/match`).
- **List:** a grid of `QuestCard`s (one column on mobile, 2–3 on desktop). Source: `db::list_pool`, 20 per page.
- **Load more:** a "Show more" button under the list. HTMX `hx-get="/?cursor={next_cursor}" hx-target="#pool" hx-swap="beforeend"`. The button disappears when there is no `next_cursor`.
- **Filters (not in v1, v2):** time (S/M/L) and energy. The API has no filters, so they wait.
- **Empty:** "The pool is empty. Drop the first quest." + button.
- **503:** a calm banner on top: "We can't reach the pool right now, try again shortly." The empty state replaces the list; in degraded mode the server already returns an empty pool.

### 5.2 Drop (`/drop`)
**Decision: dropping requires login.** The API allows anonymous drops today, but voice upload (`/v1/voice`) already needs login, and anonymous drops let anyone drop in someone else's name (design §6). After login the form still fits the 30-second rule. This decision needs the backend change in §10, item 8.

One-column form. Field order and rules must match `validate.rs` exactly:

| Field | Control | Rule (server) | Help text |
|---|---|---|---|
| Title | text | 3–80 chars | "Name of the quest" |
| One-liner | text | 10–200 chars | "Where did you stop?" |
| Link | url | http(s), ≤2048 | "Repo, doc, Figma…" |
| Voice note | recorder + file upload | §6 | "10–60 seconds: what did you want to do, where did you get stuck?" |
| Skills | chip input | ≤10, each 1–32, lowercased | "rust, illustration, copywriting…" |
| Time | 3-way segment | S / M / L | S: "under 2 hours" · M: "2–8 hours" · L: "over 8 hours" |
| Energy | 3-way segment | LOW / MID / HIGH | "Needs low / medium / high energy" |

Handle and email are not in the form; the server fills them from the session.

- Time and energy have **no default**; the user picks deliberately. Both segments are large and touch-friendly.
- **Errors:** the server returns an `errors[]` list (one line per failed rule). The UI puts each line under its field; unmapped lines go to a summary box above the form. Form data is never lost.
- **Daily cap (429 + Retry-After):** "You've dropped 3 quests today. You can drop the next one in {hours}." The submit button is disabled.
- **Success:** "Your quest is in the pool 🎯" + a preview card + "See quests for me" (`/match`). If `next` is set (e.g. coming from `/take/…`), go back there.

### 5.3 Login (`/login`)
See §4.2. One field, one button. `auth/request` always says "sent" (it doesn't reveal whether an account exists), and the UI shows the same message. In development (no SMTP) the link goes to the log; in that case a small "dev mode: the link is in the server log" note may appear at the bottom (only when `BASE_URL` is localhost).

### 5.4 Match (`/match`)
- 4 cards: the first 3 labelled "Fits you", the 4th labelled **"Surprise"** with a differently coloured frame.
- Each card says why in plain words. The API returns `reason` as `fit=0.50 rand=0.31`; the UI never shows that raw and writes "50% overlap with your skills". The surprise card says "Something completely new".
- "Refresh" button: the rule matcher is 20% random, so the order shifts slightly on every refresh.
- `?algo=embedding` is not exposed in the v1 UI.
- **Empty:** "No quest in the pool fits you right now" + "See the whole pool".

### 5.5 Swap confirmation (`/take/{offer_id}`)
- The quest to take on top (full card, voice player open), below it "Which of your quests do you give in return?" and the user's open quests.
- Explanation line: "Swaps go both ways: your quest moves to the other person too."
- The "Swap" button locks on click. The request carries an Idempotency-Key (§7.3).
- **409 cases** (by the server's `detail`):
  - "offer is no longer open" → "Someone took it before you. Look at another quest." + `/match`
  - "give offer is not yours" / "cannot take your own project" → impossible in the normal flow; show a generic error if it happens.
- **Success:** short celebration → `/revive#{match_id}`.

### 5.6 My quests / Revive (`/revive`)
- **Tab 1 – Taken** (`all_revives` data): newest first, expanded. The card shows the freeze package (title, one-liner, link, voice, skills), the giver's handle and a **First 2 minutes** box:
  1. Open the link and skim for 30 seconds
  2. Listen to the voice note end to end
  3. Reproduce the smallest `{skill}` piece locally
  4. Write down the single next step
  - The steps are checkboxes; their state lives in `localStorage` under `questdrop:revive:{match_id}` (never sent to the server).
  - "Start the 2 minutes" shows a visible countdown. At zero it says "If you want to keep going, it's your quest now."
- **Tab 2 – Dropped:** open and swapped quests. **Not in the API** (§10, item 3).
- **Empty:** "You haven't taken a quest yet. See quests for you."

### 5.7 Profile (`/me`)
- Handle and email (read-only; changing them is not in v1).
- **I can do** and **I'm looking for**: chip inputs, up to 20 each. Save → `upsert_profile`.
- My open quests (§10, item 3).
- "Log out" → logout → `/`.

### 5.8 Quest detail (`/q/{project_id}`)
A shareable single-card page with Open Graph tags (title, one-liner). If the quest is no longer open: "This quest has moved to a new owner" plus a link to the pool. **The API has no single-project endpoint**; a small query is added to the `db` layer (§10, item 6).

### 5.9 Report
Every `QuestCard` has a "⋯" menu → "Report" → a small dialog (reason, 10–300 chars). Requires login. 409 "already reported" → "You already reported this, thanks." The option is hidden on the user's own quests. If the response has `hidden: true`, the card fades out of the list.

---

## 6. Voice note (the riskiest part)

**Status (2026-10-12): solved with option A.** The backend now accepts mp3, ogg, webm and m4a. The original problem, kept for context: the backend accepted **mp3 only** (magic bytes + `.mp3` extension). Browsers do not record mp3 natively: Chrome/Firefox record Opus in webm/ogg, Safari records AAC in mp4 ([MDN `isTypeSupported`](https://developer.mozilla.org/en-US/docs/Web/API/MediaRecorder/isTypeSupported_static), [addpipe](https://blog.addpipe.com/recording-audio-in-the-browser-using-pure-html5-and-minimal-javascript)). Uploading an in-browser recording as-is got a 400.

Options:

| | Approach | Pros | Cons |
|---|---|---|---|
| A | **Backend also accepts Opus/AAC** (webm, ogg, m4a magic bytes) | Native recording, small files, little JS | Changes `validate.rs`, `looks_like_mp3` and tests; widens the `voice_url` extension rule |
| B | Encode to mp3 in the browser (lamejs-style library) | Backend unchanged | ~150 KB extra JS, slow on mobile, weakly maintained libraries |
| C | Transcode to mp3 on the server with ffmpeg | Simple client | Bigger image, CPU, new attack surface |

**Chosen: A.** The UI tries `MediaRecorder.isTypeSupported()` in order: `audio/webm;codecs=opus` → `audio/ogg;codecs=opus` → `audio/mp4`. File upload always stays available (for people with a ready audio file).

**Recorder component:**
- A big round "Record" button, with a timer and a level meter while recording.
- **Recordings under 10 seconds are rejected**, and recording stops automatically at 60 seconds. The server does not check duration, so this rule lives only in the UI (design §6).
- After recording: listen · re-record · use. "Use" uploads the file to `/v1/voice` right away with a progress bar and writes the returned `voice_url` into a hidden field. The upload must finish before the form can be submitted.
- Microphone permission denied: "No microphone access. You can upload an audio file instead."
- 413 → "The file is larger than 5 MB." · 507 → "Our voice storage is full, please tell us." · 401 → the session expired; go to login (form data is kept in `sessionStorage`).

**Player:** native `<audio controls preload="none">`. Playing an external `voice_url` (another domain) sends the listener's IP to that domain. In v1, external voice links carry a visible "external audio" label. Long term, accepting only `/voice/` is recommended.

---

## 7. Errors and states

### 7.1 HTTP → UI mapping

| Code | When | UI |
|---|---|---|
| 400 + `errors[]` | validation | messages under fields + summary box |
| 401 | no / expired session | `/login?next=…`; the filled form stays in `sessionStorage` |
| 403 | id does not match the session | generic error (shouldn't happen in the normal flow) |
| 404 | not found | in-page empty state |
| 409 | swap race, duplicate report, handle conflict | friendly text by the server's `detail` (§5.5, §5.9) |
| 413 / 507 | voice | §6 |
| 429 | IP limit or daily drop cap | countdown from `Retry-After`, button disabled |
| 503 | no database | banner on top, read screens show empty states, write buttons disabled |
| other 5xx | – | "Something went wrong, try again" + retry button |

Server messages are in English; the UI translates `detail` through a **mapping table** and falls back to a generic message for anything unmapped. Raw server text is never shown to end users.

### 7.2 HTMX error behaviour
By default HTMX does not swap 4xx/5xx responses into the page. UI handlers return **200 + an error fragment** (the form again, with errors), or use the `HX-Retarget`/`HX-Reswap` headers. The error box carries `role="alert"` for screen readers.

### 7.3 Double submits
- Every submit button is disabled while its request runs (`hx-disabled-elt`).
- The swap form carries a UUID generated at render time in a hidden field and sends it as `Idempotency-Key`. If the form is submitted again, the server replays the same result with 200 (`Idempotent-Replayed: true`), and the UI treats that as success.

---

## 8. Visual language

- **Tone:** calm, warm, slightly playful. A "quest board", not a dashboard.
- **Colour:** neutral light background + one accent colour (primary buttons) + a second accent for the surprise card. Red only for real errors. All colours are CSS variables; dark theme via `prefers-color-scheme: dark`.
- **Typography:** system fonts (`system-ui`), body 16–18 px, line height 1.5. Headings bold and short.
- **Icons:** small/medium/large hourglass for time S/M/L; a battery with 1/2/3 bars for energy LOW/MID/HIGH. **Icons always come with text**; an icon never carries meaning alone.
- **`QuestCard`:** title · one-liner · skill chips · time + energy badges · voice player · "open link" · `@giver` · primary action. Only the action button is clickable, not the whole card.
- **Motion:** a 600 ms celebration on swap, none on page transitions. Fully off under `prefers-reduced-motion`.
- **Spacing:** 8 px grid; touch targets at least 44×44 px.

---

## 9. Accessibility, language, security

**Accessibility (WCAG 2.2 AA):**
- Fully keyboard-operable; visible focus ring; "skip to content" link.
- Regions updated by HTMX use `aria-live="polite"`. Error summaries use `role="alert"`.
- Segment controls (time/energy) are real `<input type="radio">` inside `<fieldset><legend>`.
- Voice recorder: the button state (`aria-pressed`) and recording time are announced to screen readers.
- Voice notes have no text equivalent. The one-liner partly covers that; an optional "summary of the voice note" field could come in v2.
- Contrast at least 4.5:1; no horizontal scrolling at 200% zoom.

**Language:** v1 in English + Turkish. Strings live in one dictionary file per language (`i18n/en.toml`, `i18n/tr.toml`), not in templates. The language follows `Accept-Language`, and the user can switch it (cookie). User content is not translated.

**Security:**
- Askama escapes automatically; the `|safe` filter is **never** used on user content.
- `link_url` is always rendered with `rel="noopener noreferrer nofollow ugc" target="_blank"`. The server already accepts only http(s).
- **CSRF:** UI form handlers (`POST /drop`, `/take`, `/me`, `/logout`) check the `Origin`/`Sec-Fetch-Site` header; `SameSite=Lax` alone is not considered enough. A session-bound CSRF token is the alternative.
- **CSP:** `default-src 'self'; media-src 'self' https:; img-src 'self' data:; script-src 'self'; style-src 'self'; frame-ancestors 'none'`. No inline scripts (HTMX `hx-*` attributes are CSP-compatible).
- `next` only accepts paths that start with `/` and not with `//`.

---

## 10. Backend/API work needed before the UI

| # | Work | Why | Size |
|---|---|---|---|
| 1 | ✅ Backend part done: set `LOGIN_URL` to the UI's login page. The page itself (GET shows the token, POST consumes it) is UI work | the emailed link must open it | small |
| 2 | ✅ `offer_id` on pool and match items | swaps use offer ids | done |
| 3 | ✅ `GET /v1/me/offers?status=` | required by `/take` and `/revive` tab 2 | done |
| 4 | (Only if a separate-origin SPA is chosen) CORS with credentials | not needed in the same binary | small |
| 5 | ✅ Voice format: mp3, ogg, webm, m4a (§6, option A) | in-browser recording | done |
| 6 | Single-quest query (for `/q/{id}`) | shareable page | small |
| 7 | The UI calls `/match` and `/revive` with the session user; the REST side requires login | privacy gap in design §6 | small |
| 8 | Require login for drops (§5.2 decision) | dropping in someone else's name | small; tests change |
| 9 | Remove the `/v1/match` link from the home template | 400 without `user_id` | tiny |

With 1, 2, 3 and 5 done (2026-10-12), the core loop (drop → match → take) is possible once the UI exists. Item 8 is still needed for the §5.2 decision.

---

## 11. Build order (slices)

Each slice can ship on its own:

1. **Skeleton:** `base.html` (menu, theme, i18n), `static/` (CSS, htmx), CSP and security headers. Pool page (read-only, paging).
2. **Login:** `/login`, `/login?token=`, logout, `next` redirect, session helpers. (§10: 1)
3. **Drop:** form, recorder, upload, error mapping, daily cap. (§10: 5, 8)
4. **Match + take:** `/match`, `/take/{offer_id}`, Idempotency-Key, celebration. (§10: 2, 3)
5. **Revive + profile + report:** `/revive`, first 2 minutes, `/me`, report dialog. (§10: 7)
6. **Polish:** `/q/{id}` + Open Graph, dark theme, empty states, microcopy, accessibility audit. (§10: 6, 9)

---

## 12. Tests

- **E2E (Playwright, real Postgres):** two users in two browser contexts: A and B log in (token read from the log mailer) and each drop a quest, A matches, A takes B's quest, A sees the 4 steps on the revive screen. This is the plan's definition of done.
- **Race:** two users press "Swap" on the same quest at once → one succeeds, the other sees the §5.5 409 message.
- **Forms:** a field message for every validation rule; the drop form (with file upload) works with JS off.
- **Accessibility:** automated axe scan on every page; core loop by keyboard only.
- **Visual:** 360 px, 768 px, 1280 px; light/dark screenshot comparison.
- **Voice:** record → upload → play on Chrome, Firefox and Safari (including iOS).

---

## 13. Open questions

1. Require login for drops? (This document assumes yes, §5.2.)
2. ~~Voice format~~: decided and done (A, §6).
3. Disallow external voice links entirely?
4. v1 language: English only, or English + Turkish? (Recommendation: both, via dictionary files.)
5. Should the new owner of a swapped quest have a way to contact the giver? There is none today (DMs are out of scope). The giver's handle is visible, but there is no way to reach them.
