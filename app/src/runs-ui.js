// Desktop run selection. History records only successful uploads by this app.
export function installRuns({ invoke, listen, getFile, setBusy, showError, getRankingsSettings, rankingsAuthExpired }) {
  const icons = {
    scan: '<circle cx="10" cy="10" r="6"/><path d="m15 15 5 5"/>',
    upload: '<path d="M12 16V3m-5 5 5-5 5 5M4 15v5h16v-5"/>',
    open: '<path d="M14 3h7v7m0-7L10 14M10 3H3v18h18v-7"/>',
    check: '<path d="m5 12 4 4L19 6"/>',
    refresh: '<path d="M20 6v5h-5M4 18v-5h5"/><path d="M6.1 9A7 7 0 0 1 18 6l2 2M17.9 15A7 7 0 0 1 6 18l-2-2"/>'
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
  const rankingRequests = new Set();
  let history = {};
  try { const value = JSON.parse(localStorage.getItem(key)); if (value && typeof value === "object") history = value; } catch {}
  const account = () => document.getElementById("email").value.trim().toLowerCase();
  const historyKey = (run) => JSON.stringify([account(), run.id]);
  const uploaded = (run) => history[historyKey(run)];
  function saveHistory() {
    localStorage.setItem(key, JSON.stringify(history));
  }
  function reportCode(record) {
    if (record.code) return record.code;
    const match = String(record.url || "").match(/\/reports\/([A-Za-z0-9]+)/);
    return match ? match[1] : "";
  }
  function percentileClass(value) {
    if (value >= 100) return "rank-gold";
    if (value >= 99) return "rank-pink";
    if (value >= 95) return "rank-orange";
    if (value >= 75) return "rank-purple";
    if (value >= 50) return "rank-blue";
    if (value >= 25) return "rank-green";
    return "rank-grey";
  }
  function metric(labelText, value) {
    const badge = document.createElement("span");
    badge.className = "run-metric " + (Number.isFinite(value) ? percentileClass(value) : "rank-pending");
    const metricLabel = document.createElement("span"); metricLabel.textContent = labelText;
    const number = document.createElement("strong"); number.textContent = Number.isFinite(value) ? Math.round(value) + "%" : "—";
    badge.append(metricLabel, number); return badge;
  }
  function rankingsFor(run, record) {
    const settings = getRankingsSettings();
    const scores = record.rankings;
    return scores && scores.character === settings.character && scores.kind === run.kind ? scores : null;
  }
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
        const settings = getRankingsSettings();
        const scores = rankingsFor(run, record);
        const metrics = document.createElement("div"); metrics.className = "run-metrics";
        if (scores) {
          metrics.append(metric("Parse", scores.parsePercent), metric(run.kind === "raid" ? "iLvl" : "Key", scores.bracketPercent));
          const scope = run.kind === "raid" && scores.fightsRanked > 1
            ? `Average across ${scores.fightsRanked} ranked boss kills.`
            : "Percentiles supplied by Warcraft Logs.";
          metrics.title = scope;
        } else {
          const pending = document.createElement("span"); pending.className = "rankings-pending-text";
          pending.textContent = record.rankingsStatus === "loading" ? "Calculating rankings…"
            : !settings.character ? "Choose your character to load rankings"
            : !settings.accessToken ? "Connect rankings to load scores"
            : record.rankingsStatus === "error" ? "Rankings not ready yet"
            : "Rankings pending";
          metrics.append(pending);
          if (settings.character && settings.accessToken && record.rankingsStatus !== "loading") {
            const refresh = document.createElement("button"); refresh.type = "button"; refresh.className = "rankings-refresh";
            label(refresh, "Refresh", "refresh"); refresh.onclick = () => refreshRankings(run, record, 0, true);
            metrics.append(refresh);
          }
        }
        row.append(metrics);
      }
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
  async function refreshRankings(run, record, attempt = 0, manual = false) {
    const settings = getRankingsSettings();
    const code = reportCode(record);
    if (!settings.accessToken || !settings.character || !code) { render(); return; }
    const requestKey = [code, settings.character].join("|");
    if (rankingRequests.has(requestKey)) return;
    rankingRequests.add(requestKey); record.rankingsStatus = "loading"; render();
    try {
      const result = await invoke("fetch_report_rankings", {
        accessToken: settings.accessToken,
        reportCode: code,
        character: settings.character,
      });
      if (result.pending) {
        record.rankingsStatus = "pending";
        render();
        const waits = [5000, 15000, 30000, 60000];
        if (attempt < waits.length) setTimeout(() => refreshRankings(run, record, attempt + 1), waits[attempt]);
        return;
      }
      record.rankingsStatus = "done";
      record.rankings = {
        character: settings.character,
        kind: run.kind,
        parsePercent: result.parsePercent,
        bracketPercent: result.bracketPercent,
        fightsRanked: result.fightsRanked,
        fetchedAt: Date.now(),
      };
      saveHistory(); render();
    } catch (error) {
      const message = String(error);
      record.rankingsStatus = "error"; render();
      if (/authorization expired|\b40[13]\b/i.test(message)) rankingsAuthExpired();
      else if (manual) showError(message);
    } finally {
      rankingRequests.delete(requestKey);
    }
  }
  function refreshMissingRankings() {
    const settings = getRankingsSettings();
    if (!settings.accessToken || !settings.character) { render(); return; }
    for (const run of runs) {
      const record = uploaded(run);
      if (record && !rankingsFor(run, record)) refreshRankings(run, record);
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
    finally { lock(false); refreshMissingRankings(); }
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
        const record = { url: result.url, code: result.code, rankingsStatus: "pending" };
        history[JSON.stringify([email.trim().toLowerCase(), run.id])] = record;
        try { saveHistory(); }
        catch { throw new Error("Report uploaded, but history could not be saved. Report: " + result.url); }
        render();
        refreshRankings(run, record);
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
  document.addEventListener("rankings-settings-changed", () => { render(); refreshMissingRankings(); });
  render();
}
