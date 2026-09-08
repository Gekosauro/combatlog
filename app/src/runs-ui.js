// Desktop run selection. History records only successful uploads by this app.
export function installRuns({ invoke, listen, getFile, isWarcraft, setBusy, showError }) {
  const host = document.createElement("div");
  host.style.cssText = "margin-top:16px;font-size:0.8rem";
  document.getElementById("uploadFields").append(host);
  const scanButton = document.createElement("button");
  scanButton.type = "button"; scanButton.className = "copy-btn"; scanButton.textContent = "Scan raids and Mythic+ runs";
  const summary = document.createElement("p"); summary.style.margin = "12px 0";
  const list = document.createElement("div");
  const all = document.createElement("button"); all.type = "button"; all.className = "btn";
  all.textContent = "Upload all missing runs";
  host.append(scanButton, summary, list, all);
  const key = "combatlog_run_history_v1";
  let runs = [], busy = false, generation = 0, scannedPath = null;
  let history = {};
  try { const value = JSON.parse(localStorage.getItem(key)); if (value && typeof value === "object") history = value; } catch {}
  const account = () => document.getElementById("email").value.trim().toLowerCase();
  const historyKey = (run) => JSON.stringify([account(), run.id]);
  const uploaded = (run) => history[historyKey(run)];
  function render() {
    host.hidden = !isWarcraft();
    scanButton.disabled = busy || !getFile();
    all.disabled = busy || !runs.some(r => r.complete && !uploaded(r));
    list.replaceChildren();
    for (const run of runs) {
      const row = document.createElement("div"); row.style.cssText = "border-top:1px solid var(--border);padding:12px 0";
      const difficulties = { 3: "10-player", 4: "25-player", 5: "10-player Heroic", 6: "25-player Heroic", 7: "Raid Finder", 9: "40-player", 14: "Normal", 15: "Heroic", 16: "Mythic", 17: "Raid Finder" };
      const name = document.createElement("strong"); name.textContent = run.kind === "raid"
        ? `${run.name} — Raid (${difficulties[run.difficulty] || run.difficulty})`
        : `${run.name} +${run.level}`;
      const date = document.createElement("p");
      const seconds = Math.floor((run.durationMs || 0) / 1000);
      date.textContent = run.started + (run.durationMs ? ` — ${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}` : "");
      const state = document.createElement("p");
      const record = uploaded(run);
      state.textContent = !run.complete ? (run.kind === "raid"
        ? "Open / incomplete raid session — leave the raid and scan again"
        : "Incomplete / abandoned — upload unavailable") : record ? "Uploaded" : "Not uploaded by this app";
      row.append(name, date, state);
      if (record) {
        const link = document.createElement("button"); link.type = "button"; link.className = "copy-btn"; link.textContent = "Open report";
        link.onclick = () => invoke("open_url", { url: record.url }).catch(e => showError(String(e)));
        row.append(link);
      } else if (run.complete) {
        const button = document.createElement("button"); button.type = "button"; button.className = "copy-btn";
        button.textContent = "Upload this run"; button.disabled = busy;
        button.onclick = () => upload([run]); row.append(button);
      }
      list.append(row);
    }
  }
  function lock(value) { busy = value; setBusy(value); render(); }
  async function scan() {
    if (busy || !getFile() || !isWarcraft()) return;
    const path = getFile().path, ticket = ++generation;
    runs = []; scannedPath = null; lock(true); summary.textContent = "Reading log…";
    try {
      const found = await invoke("scan_runs", { path });
      if (ticket !== generation) return;
      runs = found; scannedPath = path;
      summary.textContent = `${runs.length} raid sessions and Mythic+ runs found. Raid wipes and kills stay together. Previous uploads from other modes or apps are not tracked here.`;
    } catch (error) { summary.textContent = "Scan failed"; showError(String(error)); }
    finally { lock(false); }
  }
  async function send(args) {
    let unlisten = [];
    try {
      let done, fail;
      const result = new Promise((resolve, reject) => { done = resolve; fail = reject; });
      // Install listeners before starting the backend task.
      unlisten.push(await listen("upload:done", e => done(e.payload)));
      unlisten.push(await listen("upload:error", e => fail(new Error(e.payload.message))));
      unlisten.push(await listen("upload:progress", e => { summary.textContent = e.payload.message; }));
      await invoke("start_upload", { args });
      return await result;
    } finally { unlisten.forEach(fn => fn()); }
  }
  async function upload(queue) {
    if (busy || !getFile() || getFile().path !== scannedPath) return;
    // Snapshot all settings so a batch has one consistent account/destination.
    const email = document.getElementById("email").value;
    const args = { logPath: scannedPath, email, password: document.getElementById("password").value,
      region: Number(document.getElementById("region").value), visibility: Number(document.getElementById("visibility").value),
      guildId: Number(document.getElementById("guild").value) || null, game: "warcraft" };
    lock(true);
    try {
      for (const run of queue) {
        const result = await send({ ...args, runId: run.id });
        history[JSON.stringify([email.trim().toLowerCase(), run.id])] = { url: result.url };
        try { localStorage.setItem(key, JSON.stringify(history)); }
        catch { throw new Error("Report uploaded, but history could not be saved. Report: " + result.url); }
        render();
      }
      summary.textContent = "Upload complete — each run has its own report.";
    } catch (error) { showError(String(error)); summary.textContent = "Stopped. Successful uploads are listed below; remaining runs can be retried."; }
    finally { lock(false); }
  }
  scanButton.onclick = scan;
  all.onclick = () => upload(runs.filter(r => r.complete && !uploaded(r)));
  document.addEventListener("log-selected", () => { runs = []; scannedPath = null; render(); scan(); });
  // Warcraft desktop uses run selection, preventing accidental whole-file merges.
  document.getElementById("form").addEventListener("submit", e => {
    if (!isWarcraft() || document.getElementById("uploadFields").style.display === "none") return;
    e.preventDefault(); e.stopImmediatePropagation();
    if (scannedPath) upload(runs.filter(r => r.complete && !uploaded(r))); else scan();
  }, true);
  render();
}
