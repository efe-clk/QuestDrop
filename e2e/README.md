# Browser E2E

Drives the real web UI in Chromium with Playwright: two users log in,
drop quests (one with an in-browser recording from a fake microphone,
one with a file), match, swap and revive; a third user's quest is raced
for by both; reporting, a JavaScript-off drop, axe accessibility scans
and screenshots at 360/768/1280 px in light and dark mode are included.

It runs against a live server and a **fresh** database (matching shows
only four quests, so leftovers from earlier runs can hide the expected
one). Login links are read from the server log (log mailer, no SMTP).

```bash
# 1. a throwaway database + server, logging to a file
export DATABASE_URL=postgresql://user:pass@localhost:5432/questdrop_e2e
SESSION_SECRET=e2e-secret-0123456789abcdef cargo run > /tmp/qd.log 2>&1 &

# 2. the test
cd e2e && npm install && npx playwright install chromium
SERVER_LOG=/tmp/qd.log OUT=/tmp/qd-shots npm test
```

`BASE` (default `http://localhost:3000`), `OUT` (screenshots) and
`CHROMIUM` (path to a Chromium binary) are optional.
