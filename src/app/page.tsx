export default function Home() {
  return (
    <main>
      <p>
        <a href="https://github.com/efe-clk/QuestDrop/blob/main/README.tr.md">
          🇹🇷 Türkçe için tıklayın
        </a>
      </p>
      <h1>QuestDrop</h1>
      <p>Drop your stale project. Pick up someone else&apos;s quest.</p>
      <h2>Drop (30s)</h2>
      <form>
        <input name="title" placeholder="Title" required />
        <br />
        <input name="link_url" placeholder="Link URL" type="url" required />
        <br />
        <input name="voice_url" placeholder="Voice URL (10-60s mp3)" type="url" required />
        <br />
        <select name="time_bucket" defaultValue="S">
          <option value="S">S (&lt;2h)</option>
          <option value="M">M (2-8h)</option>
          <option value="L">L (8h+)</option>
        </select>{" "}
        <select name="energy" defaultValue="LOW">
          <option value="LOW">low</option>
          <option value="MID">mid</option>
          <option value="HIGH">high</option>
        </select>
        <br />
        <button type="submit">Freeze + offer</button>
      </form>
      <h2>Pool</h2>
      <p>
        API: <a href="/api/health">/api/health</a> · <a href="/api/projects">/api/projects</a>
      </p>
    </main>
  );
}
