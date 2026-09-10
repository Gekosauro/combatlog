// Desktop run selection. History records only successful uploads by this app.
export function installRuns({ invoke, listen, getFile, setBusy, showError }) {
  const icons = {
    scan: '<circle cx="10" cy="10" r="6"/><path d="m15 15 5 5"/>',
    upload: '<path d="M12 16V3m-5 5 5-5 5 5M4 15v5h16v-5"/>',
    open: '<path d="M14 3h7v7m0-7L10 14M10 3H3v18h18v-7"/>',
    check: '<path d="m5 12 4 4L19 6"/>'
  };
  function label(button, text, icon) {
    button.innerHTML = '<svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">' + icons[icon] + '</svg>';
    button.append(document.createTextNode(text));
  }
  const host = document.createElement("div");
  host.className = "runs-panel";
  document.getElementById("uploadFields").append(host);
  const scanButton = document.createElement("button");
  scanButton.type = "button"; scanButton.className = "run-action scan-action"; label(scanButton, "Scan raids & Mythic+", "scan");
  const summary = document.createElement("p"); summary.className = "runs-summary"; summary.setAttribute("role", "status"); summary.textContent = "Choose a log to discover your runs.";
  const list = document.createElement("div"); list.className = "runs-list";
  const all = document.createElement("button"); all.type = "button"; all.className = "run-action batch-action";
  label(all, "Upload all missing runs", "upload");
  const activity = document.createElement("section"); activity.className = "run-activity"; activity.hidden = true;
  const activityTitle = document.createElement("strong");
  const activityStatus = document.createElement("p"); activityStatus.setAttribute("role", "status");
  const meter = document.createElement("progress"); meter.max = 100; meter.value = 0; meter.setAttribute("aria-label", "Current run upload progress");
  const details = document.createElement("details");
  const detailsTitle = document.createElement("summary"); detailsTitle.textContent = "Upload details";
  const output = document.createElement("pre");
  details.append(detailsTitle, output);
  activity.append(activityTitle, meter, activityStatus, details);
  host.append(scanButton, summary, all, activity, list);
  function updateActivity(message, pct) {
    activityStatus.textContent = message;
    if (Number.isFinite(pct)) meter.value = Math.max(meter.value, Math.min(100, pct));
    output.textContent = (output.textContent + message + "\n").slice(-12000);
    output.scrollTop = output.scrollHeight;
  }
  const key = "combatlog_run_history_v1";
  let runs = [], busy = false, generation = 0, scannedPath = null;
  let history = {};
  try { const value = JSON.parse(localStorage.getItem(key)); if (value && typeof value === "object") history = value; } catch {}
  const account = () => document.getElementById("email").value.trim().toLowerCase();
  const historyKey = (run) => JSON.stringify([account(), run.id]);
  const uploaded = (run) => history[historyKey(run)];
  function render() {
    scanButton.disabled = busy || !getFile();
    const missing = runs.filter(r => r.complete && !uploaded(r)).length;
    all.disabled = busy || !missing;
    label(all, "Upload all missing runs" + (missing ? " (" + missing + ")" : ""), "upload");
    list.replaceChildren();
    for (const run of runs) {
      const row = document.createElement("article"); row.className = "run-card";
      const info = document.createElement("div"); info.className = "run-info";
      const difficulties = { 3: "10-player", 4: "25-player", 5: "10-player Heroic", 6: "25-player Heroic", 7: "Raid Finder", 9: "40-player", 14: "Normal", 15: "Heroic", 16: "Mythic", 17: "Raid Finder" };
      const name = document.createElement("strong"); name.textContent = run.kind === "raid"
        ? `${run.name} — Raid (${difficulties[run.difficulty] || run.difficulty})`
        : `${run.name} +${run.level}`;
      const date = document.createElement("p"); date.className = "run-date";
      const seconds = Math.floor((run.durationMs || 0) / 1000);
      date.textContent = run.started + (run.durationMs ? ` — ${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}` : "");
      const state = document.createElement("p"); state.className = "run-state";
      const record = uploaded(run);
      state.textContent = !run.complete ? (run.kind === "raid"
        ? "Open / incomplete raid session — leave the raid and scan again"
        : "Incomplete / abandoned — upload unavailable") : record ? "Uploaded" : "Not uploaded by this app";
      if (record) { row.classList.add("is-uploaded"); label(state, "Uploaded", "check"); }
      info.append(name, date, state); row.append(info);
      if (record) {
        const link = document.createElement("button"); link.type = "button"; link.className = "run-action report-action"; label(link, "Open report", "open");
        link.onclick = () => invoke("open_url", { url: record.url }).catch(e => showError(String(e)));
        row.append(link);
      } else if (run.complete) {
        const button = document.createElement("button"); button.type = "button"; button.className = "run-action upload-action";
        label(button, "Upload this run", "upload"); button.disabled = busy;
        button.onclick = () => upload([run]); row.append(button);
      }
      list.append(row);
    }
  }
  function lock(value) { busy = value; setBusy(value); render(); }
  async function scan() {
    if (busy || !getFile()) return;
    const path = getFile().path, ticket = ++generation;
    runs = []; scannedPath = null; activity.hidden = true; lock(true); summary.textContent = "Reading log…";
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
      unlisten.push(await listen("upload:progress", e => { updateActivity(e.payload.message, e.payload.pct); }));
      await invoke("start_upload", { args });
      return await result;
    } finally { unlisten.forEach(fn => fn()); }
  }
  async function upload(queue) {
    if (busy || !queue.length || !getFile() || getFile().path !== scannedPath) return;
    // Snapshot all settings so a batch has one consistent account/destination.
    const email = document.getElementById("email").value;
    const args = { logPath: scannedPath, email, password: document.getElementById("password").value,
      region: Number(document.getElementById("region").value), visibility: Number(document.getElementById("visibility").value),
      guildId: Number(document.getElementById("guild").value) || null };
    lock(true);
    activity.hidden = false; activity.dataset.state = "uploading";
    output.textContent = ""; document.getElementById("error-msg").style.display = "none";
    try {
      for (const [index, run] of queue.entries()) {
        activityTitle.textContent = "Uploading " + (index + 1) + " of " + queue.length + " · " + run.name;
        meter.value = 0; updateActivity("Preparing separate report…", 0);
        const result = await send({ ...args, runId: run.id });
        history[JSON.stringify([email.trim().toLowerCase(), run.id])] = { url: result.url };
        try { localStorage.setItem(key, JSON.stringify(history)); }
        catch { throw new Error("Report uploaded, but history could not be saved. Report: " + result.url); }
        render();
      }
      activity.dataset.state = "done"; activityTitle.textContent = "Upload complete";
      updateActivity(queue.length + " separate report" + (queue.length === 1 ? "" : "s") + " uploaded successfully.", 100);
    } catch (error) { activity.dataset.state = "error"; activityTitle.textContent = "Upload stopped"; updateActivity(String(error)); }
    finally { lock(false); }
  }
  scanButton.onclick = scan;
  all.onclick = () => upload(runs.filter(r => r.complete && !uploaded(r)));
  document.addEventListener("log-selected", () => { runs = []; scannedPath = null; render(); scan(); });
  // Warcraft desktop uses run selection, preventing accidental whole-file merges.
  document.getElementById("form").addEventListener("submit", e => {
    if (document.getElementById("uploadFields").style.display === "none") return;
    e.preventDefault(); e.stopImmediatePropagation();
    // Enter in a form field never starts an unintended batch upload.
    if (!scannedPath) scan();
  }, true);
  document.getElementById("email").addEventListener("change", render);
  render();
}
