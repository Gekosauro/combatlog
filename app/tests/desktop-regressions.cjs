const { test } = require("node:test");
const assert = require("node:assert/strict");
const { readFileSync, existsSync } = require("node:fs");
const { resolve } = require("node:path");
const vm = require("node:vm");
const root = resolve(__dirname, "..");
const read = path => readFileSync(resolve(root, path), "utf8").replace(/\r\n/g, "\n");
const html = read("src/index.html");
const runs = read("src/runs-ui.js");

test("rankings UI and backend commands are removed", () => {
  assert.doesNotMatch(html, /Performance rankings|performanceCharacter|connectRankings|rankings_config/);
  assert.doesNotMatch(runs, /fetch_report_rankings|refreshRankings|rankings-settings-changed/);
  assert.doesNotMatch(read("src-tauri/src/main.rs"), /mod rankings|connect_rankings|fetch_report_rankings/);
  assert.equal(existsSync(resolve(root, "src-tauri/src/rankings.rs")), false);
});

test("Log-O-Matic branding, mascot, app identity and version stay consistent", () => {
  const config = JSON.parse(read("src-tauri/tauri.conf.json"));
  assert.equal(config.productName, "Log-O-Matic");
  assert.equal(config.identifier, "dev.combatlog.app");
  assert.match(html, /<title>Log-O-Matic<\/title>/);
  assert.match(html, /assets\/log-o-matic-mascot.png/);
  assert.equal(existsSync(resolve(root, "src/assets/log-o-matic-mascot.png")), true);
  assert.ok(read("src-tauri/Cargo.toml").includes('version = "' + config.version + '"'));
  assert.ok(read("src-tauri/Cargo.lock").includes('name = "combatlog"\nversion = "' + config.version + '"'));
});

test("legacy OAuth cleanup preserves credentials, run history and unrelated data", () => {
  const entries = {
    combatlog_rankings_v1: '{"token":"old-token"}',
    wcl_characters_warcraft_test: '["Test-Realm-EU"]',
    wcl_upload_creds: '{"email":"test@example.invalid"}',
    combatlog_run_history_v1: '{"run":{"url":"https://www.warcraftlogs.com/reports/TEST","rankings":{"parsePercent":80}}}',
    combatlog_incremental_upload_v2: '{"offset":100}',
    theme: "dark"
  };
  const before = { ...entries };
  Object.defineProperty(entries, "removeItem", { value(key) { delete entries[key]; } });
  const cleanup = html.slice(html.indexOf("      // Retire only"), html.indexOf('      const STORE_KEY ='));
  assert.ok(cleanup.length > 0);
  vm.runInNewContext(cleanup, { localStorage: entries });
  assert.equal(entries.combatlog_rankings_v1, undefined);
  assert.equal(entries.wcl_characters_warcraft_test, undefined);
  for (const key of ["wcl_upload_creds", "combatlog_run_history_v1", "combatlog_incremental_upload_v2", "theme"]) {
    assert.equal(entries[key], before[key]);
  }
  assert.doesNotThrow(() => vm.runInNewContext(cleanup, { localStorage: { removeItem() { throw Error("Storage blocked"); } } }));
});

test("separate upload loop and previous report links remain available", () => {
  assert.match(runs, /combatlog_run_history_v1/);
  assert.match(runs, /Open report/);
  assert.match(runs, /await send\(\{ \.\.\.args, runId: run.id \}\)/);
  assert.match(runs, /queue.length/);
});
