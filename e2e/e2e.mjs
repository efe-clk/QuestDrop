// QuestDrop E2E (docs/web-ui.md §12): two real users in two browser
// contexts against a running server + real Postgres. Login tokens are read
// from the LogMailer lines in the server log. See e2e/README.md.
import { chromium } from "playwright";
import AxeBuilder from "@axe-core/playwright";
import fs from "node:fs";
import path from "node:path";

const BASE = process.env.BASE || "http://localhost:3000";
const LOG = process.env.SERVER_LOG;
const OUT = process.env.OUT || "./shots";
fs.mkdirSync(OUT, { recursive: true });
// Upload fixtures: the server sniffs magic bytes only, so tiny files do.
const FIX = path.join(OUT, "fixtures");
fs.mkdirSync(FIX, { recursive: true });
fs.writeFileSync(path.join(FIX, "note.mp3"), Buffer.concat([Buffer.from("ID3\x04\x00\x00\x00\x00\x00\x00", "latin1"), Buffer.alloc(2048)]));
fs.writeFileSync(path.join(FIX, "note.ogg"), Buffer.concat([Buffer.from("OggS"), Buffer.alloc(2048)]));
const run = Date.now().toString(36);
let failures = 0;
const ok = (cond, msg) => {
  if (cond) console.log("  ✓", msg);
  else { failures++; console.log("  ✗", msg); }
};

function tokenLink(email) {
  const log = fs.readFileSync(LOG, "utf8").replace(/\x1b\[[0-9;]*m/g, "");
  const re = new RegExp(`email=${email.replace(/[.+]/g, "\\$&")}\\s+link=(\\S+)`, "g");
  let m, last = null;
  while ((m = re.exec(log))) last = m[1];
  if (!last) throw new Error("no login link for " + email);
  return last.replace(/\x1b\[[0-9;]*m/g, "");
}

const consoleErrors = [];
function watch(page, who) {
  page.on("console", (m) => { if (m.type() === "error") consoleErrors.push(`${who} @ ${page.url()}: ${m.text().slice(0, 90)}`); });
  page.on("pageerror", (e) => consoleErrors.push(`${who}: ${e.message}`));
}

async function axe(page, label) {
  const r = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "wcag21aa", "wcag22aa"]).analyze();
  const bad = r.violations.filter((v) => v.impact === "serious" || v.impact === "critical");
  ok(bad.length === 0, `axe ${label}: ${bad.length ? bad.map((v) => v.id + "(" + v.nodes.length + ")").join(", ") : "no serious/critical"}`);
  if (bad.length) for (const v of bad) console.log("     ", v.id, v.nodes[0]?.target?.join(" "), v.nodes[0]?.failureSummary?.split("\n")[1]);
}

async function login(page, email, handle, next) {
  await page.goto(BASE + "/login" + (next ? "?next=" + encodeURIComponent(next) : ""));
  await page.fill("#email", email);
  if (handle) {
    await page.click(".disclose__s");
    await page.fill("#handle", handle);
  }
  await page.click("form.form button[type=submit]");
  await page.waitForSelector("text=/Check your inbox|Gelen kutunu/");
  const link = tokenLink(email);
  await page.goto(link);
  // GET must not burn the token: the confirm button is shown first.
  await page.click("form[action='/login/confirm'] button");
  await page.waitForLoadState("networkidle");
}

async function fillDrop(page, { title, one, link, skills, tb, en, file }) {
  await page.fill("#f-title", title);
  await page.fill("#f-one", one);
  await page.fill("#f-link", link);
  for (const s of skills) {
    await page.fill(".chipin__text", s);
    await page.keyboard.press("Enter");
  }
  await page.check(`input[name=time_bucket][value=${tb}]`);
  await page.check(`input[name=energy][value=${en}]`);
  if (file) {
    await page.setInputFiles("#f-voice-file", file);
    await page.waitForSelector("[data-rec-done]:not([hidden])", { timeout: 10000 });
  }
}

const browser = await chromium.launch({
  executablePath: process.env.CHROMIUM || undefined,
  args: ["--use-fake-ui-for-media-stream", "--use-fake-device-for-media-stream"],
});
const mk = async (opts = {}) => {
  const ctx = await browser.newContext({ viewport: { width: 1280, height: 900 }, locale: "en-US", ...opts });
  await ctx.grantPermissions(["microphone"], { origin: BASE });
  await ctx.addInitScript(() => {
    document.addEventListener("securitypolicyviolation", (e) => {
      const t = e.target && e.target.outerHTML ? e.target.outerHTML.slice(0, 160) : String(e.target);
      console.error("CSPV " + e.violatedDirective + " line " + e.lineNumber + " src " + e.sourceFile + " el " + t);
    });
  });
  return ctx;
};

try {
  const A = await mk(), B = await mk({ locale: "tr-TR", extraHTTPHeaders: { "Accept-Language": "tr-TR,tr" } });
  const pa = await A.newPage(), pb = await B.newPage();
  watch(pa, "A"); watch(pb, "B");
  const ea = `a_${run}@x.com`, eb = `b_${run}@x.com`;

  console.log("1. First visit → drop needs login → back to /drop");
  await pa.goto(BASE + "/");
  await axe(pa, "pool (logged out)");
  await pa.click(".hero a[href='/drop']");
  await pa.waitForURL(/\/login\?next=%2Fdrop/);
  ok(true, "redirected to /login?next=/drop");
  await axe(pa, "login");
  await pa.fill("#email", ea);
  await pa.click(".disclose__s");
  await pa.fill("#handle", `alice_${run}`.slice(0, 24));
  await pa.click("form.form button[type=submit]");
  await pa.waitForSelector("text=Check your inbox");
  ok(await pa.locator("[data-countdown]").isDisabled(), "resend button waits 60 s");
  await pa.goto(tokenLink(ea));
  ok(await pa.locator("text=Log in to QuestDrop").isVisible(), "token GET shows a confirm button");
  await pa.click("form[action='/login/confirm'] button");
  await pa.waitForURL(/\/drop$/);
  ok(true, "landed on /drop after login");
  ok((await pa.locator(".who").textContent()).includes("@alice_"), "header shows @handle");

  console.log("2. A records a voice note in the browser (fake mic) and drops");
  await axe(pa, "drop (empty)");
  await pa.click("form#drop-form button[type=submit]");
  await pa.waitForSelector(".alert[role=alert]");
  ok(await pa.locator(".field--err").count() >= 5, "empty submit shows per-field errors");
  await axe(pa, "drop (errors)");
  await pa.screenshot({ path: `${OUT}/drop-errors-1280.png`, fullPage: true });
  await pa.click("[data-rec-toggle]");
  await pa.waitForTimeout(4000);
  await pa.click("[data-rec-toggle]");
  await pa.waitForSelector("[data-rec-status].is-err", { timeout: 5000 }).catch(() => {});
  ok((await pa.locator("[data-rec-status]").textContent()).includes("under 10 seconds"), "recording under 10 s is refused");
  await pa.click("[data-rec-toggle]");
  await pa.waitForTimeout(11000);
  await pa.click("[data-rec-toggle]");
  await pa.waitForSelector("[data-rec-review]:not([hidden])");
  await pa.click("[data-rec-use]");
  await pa.waitForSelector("[data-rec-done]:not([hidden])", { timeout: 10000 });
  const vurl = await pa.inputValue("input[name=voice_url]");
  ok(/^\/voice\/.+\.(webm|ogg|m4a)$/.test(vurl), `recording uploaded as ${vurl}`);
  await fillDrop(pa, { title: "Photo renamer CLI", one: "Renames photos by EXIF date; the config file part is missing", link: "https://github.com/x/renamer", skills: ["rust", "cli"], tb: "S", en: "LOW" });
  await pa.screenshot({ path: `${OUT}/drop-filled-1280.png`, fullPage: true });
  await pa.click("form#drop-form button[type=submit]");
  await pa.waitForURL(/\/q\/[0-9a-f-]+\?dropped=1/);
  ok(await pa.locator("text=Your quest is in the pool").isVisible(), "drop success screen");
  await axe(pa, "quest (dropped)");

  console.log("3. B (Turkish) logs in, sets skills, drops with a file upload");
  await login(pb, eb, `bora_${run}`.slice(0, 24), "/me");
  await pb.waitForURL(/\/me$/);
  ok(await pb.locator("h1", { hasText: "Profilin" }).isVisible(), "TR profile page via Accept-Language");
  await pb.fill("#can_do", "rust");
  await pb.keyboard.press("Enter");
  await pb.click("form[action='/me'] button[type=submit]");
  await pb.waitForURL(/\/me\?saved=1/);
  ok(await pb.locator("text=Kaydedildi.").isVisible(), "skills saved");
  await axe(pb, "me");
  await pb.goto(BASE + "/drop");
  await fillDrop(pb, { title: "Kedi besleme botu", one: "Discord botu, zamanlayıcı kısmı yarım kaldı", link: "https://github.com/x/kedi", skills: ["python", "discord"], tb: "M", en: "MID", file: path.join(FIX, "note.ogg") });
  await pb.click("form#drop-form button[type=submit]");
  await pb.waitForURL(/\/q\/.+\?dropped=1/);
  ok(await pb.locator("text=Quest'in havuzda").isVisible(), "B dropped (TR)");

  console.log("4. Pool lists both; A matches, takes B's quest, lands on revive");
  await pa.goto(BASE + "/");
  ok(await pa.locator(".card").count() >= 2, "pool shows cards");
  await pa.goto(BASE + "/match");
  await axe(pa, "match");
  const bCard = pa.locator(".card", { hasText: "Kedi besleme botu" });
  ok(await bCard.count() === 1, "A sees B's quest in matches");
  ok(await pa.locator(".card", { hasText: "Photo renamer CLI" }).count() === 0, "own quest excluded");
  await pa.locator(".alert--calm", { hasText: "Add your skills" }).isVisible().then((v) => ok(v, "no-skills hint shown"));
  await bCard.locator("a", { hasText: "Take this" }).click();
  await pa.waitForURL(/\/take\//);
  await axe(pa, "take");
  ok(await pa.locator("input[name=give_offer_id]").isChecked(), "single open quest preselected");
  await pa.screenshot({ path: `${OUT}/take-1280.png`, fullPage: true });
  await pa.click("form.give button[type=submit]");
  await pa.waitForURL(/\/revive\?swapped=/);
  ok(await pa.locator(".celebrate").isVisible(), "celebration shown");
  ok(await pa.locator(".revive[open] .step").count() === 4, "first 2 minutes: 4 steps");
  ok((await pa.locator(".revive[open] .step").nth(2).textContent()).includes("discord") || (await pa.locator(".revive[open] .step").nth(2).textContent()).includes("python"), "step 3 names a skill");
  await axe(pa, "revive");
  await pa.screenshot({ path: `${OUT}/revive-1280.png`, fullPage: true });
  await pa.locator(".revive[open] .step__l").first().click();
  await pa.reload();
  ok(await pa.locator(".revive[open] input[data-step='1']").isChecked(), "checklist persists in localStorage");
  await pa.click("[data-timer-start]");
  await pa.waitForTimeout(1200);
  ok((await pa.locator("[data-timer-out]").textContent()).includes("left"), "2-minute timer runs");
  await pa.goto(BASE + "/revive?tab=given");
  ok(await pa.locator(".status--MATCHED").count() === 1, "given tab: A's quest continues with someone else");

  console.log("5. B sees the swap from the other side");
  await pb.goto(BASE + "/revive?tab=given");
  ok(await pb.locator(".status--MATCHED").count() === 1, "B's quest marked as passed on");
  await pb.goto(BASE + "/revive");
  ok(await pb.locator(".empty").isVisible(), "B took nothing yet → empty state");

  console.log("6. Race: A and B tap Swap on C's quest at the same time");
  const C = await mk(); const pc = await C.newPage(); watch(pc, "C");
  const ec = `c_${run}@x.com`;
  await login(pc, ec, `cem_${run}`.slice(0, 24), "/drop");
  await fillDrop(pc, { title: "Race target", one: "A quest two people want at the same time", link: "https://example.com/race", skills: [], tb: "L", en: "HIGH", file: path.join(FIX, "note.mp3") });
  await pc.click("form#drop-form button[type=submit]");
  await pc.waitForURL(/\/q\//);
  const offer = await pc.evaluate(async () => {
    const r = await fetch("/v1/me/offers"); const j = await r.json(); return j.offers[0].offer_id;
  });
  for (const [p, t] of [[pa, "A second"], [pb, "B second"]]) {
    await p.goto(BASE + "/drop");
    await fillDrop(p, { title: t, one: "Another quest so I have something to give", link: "https://example.com/" + t.replace(" ", ""), skills: [], tb: "S", en: "LOW", file: path.join(FIX, "note.mp3") });
    await p.click("form#drop-form button[type=submit]");
    await p.waitForURL(/\/q\//);
    await p.goto(BASE + "/take/" + offer);
  }
  await Promise.all([pa.click("form.give button[type=submit]"), pb.click("form.give button[type=submit]")]);
  await Promise.all([pa.waitForLoadState("networkidle"), pb.waitForLoadState("networkidle")]);
  const won = [pa.url(), pb.url()].filter((u) => u.includes("/revive?swapped=")).length;
  ok(won === 1, `exactly one winner (${won})`);
  const loser = pa.url().includes("/revive") ? pb : pa;
  ok(await loser.locator("text=/Someone took it before you|Biri senden önce aldı/").isVisible(), "loser sees the friendly 409 message");

  console.log("7. Report flow (HTMX fragment)");
  await pc.goto(BASE + "/");
  const card = pc.locator(".card", { hasText: /A second|B second/ }).first();
  if (await card.count()) {
    await card.locator(".menu__btn").click();
    await card.locator("textarea").fill("short");
    // Native validation blocks this; strip it to exercise the server path.
    await card.locator("textarea").evaluate((t) => { t.removeAttribute("minlength"); t.removeAttribute("required"); });
    await card.locator(".report button").click();
    await pc.waitForSelector(".err[role=alert]");
    ok(true, "too-short reason shows inline error");
    await card.locator("textarea").fill("This link is broken and points nowhere useful");
    await card.locator(".report button").click();
    await pc.waitForSelector(".report__done");
    ok((await card.locator(".report__done").textContent()).includes("Thanks"), "report thanks shown in place");
  } else ok(false, "card for report not found");

  console.log("8. No-JS: login + drop with a file in the form");
  const N = await mk({ javaScriptEnabled: false }); const pn = await N.newPage();
  const en_ = `n_${run}@x.com`;
  await pn.goto(BASE + "/login?next=%2Fdrop");
  await pn.fill("#email", en_);
  await pn.click("form.form button[type=submit]");
  await pn.goto(tokenLink(en_));
  await pn.click("form[action='/login/confirm'] button");
  await pn.waitForURL(/\/drop$/);
  await pn.fill("#f-title", "No JS quest");
  await pn.fill("#f-one", "Submitted with JavaScript turned off entirely");
  await pn.fill("#f-link", "https://example.com/nojs");
  await pn.fill("#f-skills", "html, css");
  await pn.check("input[name=time_bucket][value=M]");
  await pn.check("input[name=energy][value=LOW]");
  await pn.setInputFiles("#f-voice-file", path.join(FIX, "note.mp3"));
  await pn.click("form#drop-form button[type=submit]");
  await pn.waitForURL(/\/q\/.+\?dropped=1/);
  ok(await pn.locator("text=Your quest is in the pool").isVisible(), "no-JS drop with file upload works");
  ok((await pn.locator(".card .chip").allTextContents()).join(",") === "css,html", "comma skills parsed");
  await pn.goto(BASE + "/?nope");
  await pn.goto(BASE + "/does-not-exist");
  ok(await pn.locator("text=Nothing here").isVisible(), "HTML 404 page");

  console.log("9. Screens at 360 / 768 / 1280 in light and dark");
  for (const scheme of ["light", "dark"]) {
    for (const w of [360, 768, 1280]) {
      const ctx = await browser.newContext({ viewport: { width: w, height: 900 }, colorScheme: scheme, locale: "tr-TR", extraHTTPHeaders: { "Accept-Language": "tr" } });
      const cookies = await A.cookies();
      await ctx.addCookies(cookies);
      const p = await ctx.newPage();
      watch(p, `shot-${w}-${scheme}`);
      for (const [name, url] of [["pool", "/"], ["drop", "/drop"], ["match", "/match"], ["revive", "/revive"]]) {
        await p.goto(BASE + url);
        await p.waitForLoadState("networkidle");
        const sw = await p.evaluate(() => document.documentElement.scrollWidth);
        ok(sw <= w, `${name} ${w}px ${scheme}: no horizontal scroll (${sw})`);
        await p.screenshot({ path: `${OUT}/${name}-${w}-${scheme}.png`, fullPage: true });
      }
      if (w === 360 && scheme === "dark") await axe(p, "revive dark 360");
      await ctx.close();
    }
  }

  ok(consoleErrors.length === 0, `no console errors${consoleErrors.length ? ": " + consoleErrors.slice(0, 5).join(" | ") : ""}`);
} catch (e) {
  failures++;
  console.error("E2E crashed:", e);
} finally {
  await browser.close();
}
console.log(failures ? `\n${failures} check(s) failed` : "\nall checks passed");
process.exit(failures ? 1 : 0);
