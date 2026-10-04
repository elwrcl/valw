# valw Noctalia plugin Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A Noctalia plugin (bar widget, toolbar panel, control-center tile) that drives valw through three new `valw toolbar` flags, installed by the home-manager module.

**Architecture:** valw gains `valw toolbar --state | --set KEY=VALUE | --run [MODE]` over the existing `toolbar.toml`, so the plugin holds no capture logic. The plugin is plain Luau in `noctalia-plugin/`, drawn entirely with Noctalia's components and colour roles; its logic is tested with the `luau` CLI against stubbed Noctalia globals. The nix package ships the folder; `programs.valw.noctalia.enable` links it into `$XDG_DATA_HOME/noctalia/plugins/valw`.

**Tech Stack:** Rust 2024, clap 4 (derive, `ValueEnum`), serde_json, toml; Luau (Noctalia 5.2, `plugin_api = 3`), `luau`/`luau-compile` 0.738 from nixpkgs; flake-parts (dendritic `*.ulu.nix`), home-manager module.

**Spec:** `docs/superpowers/specs/2026-10-04-valw-noctalia-plugin-design.md`

## Global Constraints

- Version control: `jj` only, never `git`. Commit with explicit paths: `jj commit -m "<msg>" <paths…>`.
- Rust checks: `cargo clippy --all-targets -- -D warnings` (never `cargo check`), `cargo nextest run` (never `cargo test`), `cargo fmt --check`.
- Nix: nix3 commands only (`nix flake check`, `nix build`, `nix shell nixpkgs#luau -c …`); no `find` over `/nix/store`; never write a manual `imports = [ … ]` list in a `*.ulu.nix` file.
- Code, comments, docs and plugin strings in plain English (plus a Turkish translation file); match the surrounding code's comment density and idiom.
- Plugin id `elars/valw`, `plugin_api = 3`, `license = "MIT"`, `dependencies = ["valw"]`, icon `camera`; entries: widget `bar`, panel `toolbar`, shortcut `tile`.
- The plugin never uses hex colours, only Noctalia role names (`primary`, `on_surface`, `outline`, `error`, …).
- No user text is ever interpolated into a `noctalia.runAsync` command string; commands are fixed strings plus known mode/key/value tokens.
- `--set` errors exit 1 like every valw error; nothing is written unless every pair is valid.
- Mode clicks and the tile wait `sleep 0.3` before capturing, so Noctalia's panel or control center is not in the shot.
- Anything touching the live session (loading the plugin into the running Noctalia, opening its panel) is announced to the user first. Edits to `~/copland` need the user's approval; the user runs rebuilds. Never include `~/copland/modules/nixos/common/common-packages.ulu.nix` in a commit.

## Review Focus

1. valw not on Noctalia's `PATH` (or `--state` failing): the panel shows "valw is not available" and an error notification, never a blank or crashed panel. — Luau test in Task 3.
2. A capture that fails (exit 1) raises a notification with stderr's last line; a cancel (exit 3) stays silent. — Luau tests in Task 3 (panel, bar, tile).
3. The panel or control center ending up in the shot: a mode click must close the panel *before* starting `sleep 0.3 && valw toolbar --run <mode>`. — Luau test in Task 3 checks the order and the command.
4. A missing or broken `toolbar.toml`: `--state` prints the defaults, `--set` starts from the defaults and leaves a valid file. — Rust tests in Task 1.
5. Anything on stdout besides the JSON line breaks the plugin's `json.decode`: `valw toolbar --state` prints exactly one JSON line (logs go to stderr and the log file). — Task 2 runs the real binary and parses its stdout.

---

### Task 1: Toolbar state bridge (`to_json`, `apply_sets`, `set`)

**Files:**
- Modify: `src/toolbar/state.rs` (the `Mode` derive at the top; new functions after `save`; tests at the bottom)

**Interfaces:**
- Consumes: existing `ToolbarState`, `Mode`, `TIMERS`, `defaults`, `load`, `save` in `src/toolbar/state.rs`.
- Produces:
  - `Mode` derives `clap::ValueEnum` (values `screen`, `window`, `region`, `zoom`).
  - `pub fn to_json(state: &ToolbarState) -> String` — one JSON line: `mode`, `timer`, `cursor`, `preview`, `sound`, `timers`.
  - `pub fn apply_sets(state: ToolbarState, sets: &[String]) -> anyhow::Result<ToolbarState>`
  - `pub fn set(path: &Path, config: &Config, sets: &[String]) -> anyhow::Result<()>`

- [ ] **Step 1: Write the failing tests**

Append inside `mod tests` at the bottom of `src/toolbar/state.rs`:

```rust
    #[test]
    fn json_for_the_plugin() {
        let state = ToolbarState {
            mode: Mode::Zoom,
            timer: 5,
            cursor: true,
            preview: false,
            sound: true,
        };
        let text = to_json(&state);
        assert!(!text.contains('\n'), "one line");
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "mode": "zoom",
                "timer": 5,
                "cursor": true,
                "preview": false,
                "sound": true,
                "timers": [0, 5, 10],
            })
        );
    }

    #[test]
    fn sets_apply_every_key_in_order() {
        let start = defaults(&Config::default());
        let sets: Vec<String> = ["mode=window", "timer=10", "cursor=true", "preview=false", "sound=false"]
            .map(String::from)
            .into();
        assert_eq!(
            apply_sets(start, &sets).unwrap(),
            ToolbarState {
                mode: Mode::Window,
                timer: 10,
                cursor: true,
                preview: false,
                sound: false,
            }
        );
        let twice = ["timer=5".to_string(), "timer=0".to_string()];
        assert_eq!(apply_sets(start, &twice).unwrap().timer, 0, "the last one wins");
    }

    #[test]
    fn bad_sets_are_rejected_by_name() {
        let start = defaults(&Config::default());
        for (pair, message) in [
            ("colour=red", "unknown toolbar key \"colour\""),
            ("mode=video", "mode must be screen, window, region or zoom"),
            ("timer=7", "timer must be 0, 5 or 10"),
            ("timer=soon", "timer must be 0, 5 or 10"),
            ("cursor=maybe", "cursor must be true or false"),
            ("cursor", "expected KEY=VALUE"),
        ] {
            let err = format!("{:#}", apply_sets(start, &[pair.to_string()]).unwrap_err());
            assert!(err.contains(message), "{pair}: {err}");
        }
    }

    #[test]
    fn set_writes_all_or_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("valw/toolbar.toml");
        let config = Config::default();
        let bad = ["timer=5".to_string(), "mode=video".to_string()];
        assert!(set(&path, &config, &bad).is_err());
        assert!(!path.exists(), "nothing written");

        let good = ["timer=5".to_string(), "sound=false".to_string()];
        set(&path, &config, &good).unwrap();
        let state = load(&path, &config);
        assert_eq!((state.timer, state.sound, state.mode), (5, false, Mode::Region));

        std::fs::write(&path, "not toml [").unwrap();
        set(&path, &config, &["cursor=true".to_string()]).unwrap();
        assert_eq!(
            load(&path, &config),
            ToolbarState {
                cursor: true,
                ..defaults(&config)
            },
            "a broken file is replaced, starting from the defaults"
        );
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo nextest run -E 'test(toolbar::state)'`
Expected: compile error, `cannot find function \`to_json\`` (and `apply_sets`, `set`).

- [ ] **Step 3: Implement**

In `src/toolbar/state.rs`, change the `Mode` derive line to:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
```

Add after `pub fn save(…)`:

```rust
/// One line of JSON for the Noctalia plugin: the state and the timer choices.
pub fn to_json(state: &ToolbarState) -> String {
    serde_json::json!({
        "mode": state.mode,
        "timer": state.timer,
        "cursor": state.cursor,
        "preview": state.preview,
        "sound": state.sound,
        "timers": TIMERS,
    })
    .to_string()
}

/// `state` with every `KEY=VALUE` applied in order; the first bad pair is
/// an error naming it.
pub fn apply_sets(mut state: ToolbarState, sets: &[String]) -> Result<ToolbarState> {
    for pair in sets {
        let Some((key, value)) = pair.split_once('=') else {
            bail!("expected KEY=VALUE, not {pair:?}");
        };
        let flag = || match value {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err(anyhow::anyhow!("{key} must be true or false, not {value:?}")),
        };
        match key {
            "mode" => {
                state.mode = <Mode as clap::ValueEnum>::from_str(value, false).map_err(|_| {
                    anyhow::anyhow!("mode must be screen, window, region or zoom, not {value:?}")
                })?;
            }
            "timer" => {
                state.timer = value
                    .parse()
                    .ok()
                    .filter(|t| TIMERS.contains(t))
                    .with_context(|| format!("timer must be 0, 5 or 10, not {value:?}"))?;
            }
            "cursor" => state.cursor = flag()?,
            "preview" => state.preview = flag()?,
            "sound" => state.sound = flag()?,
            _ => bail!("unknown toolbar key {key:?} (mode, timer, cursor, preview or sound)"),
        }
    }
    Ok(state)
}

/// `valw toolbar --set`: `sets` applied to the remembered state and saved;
/// nothing changes if any pair is bad.
pub fn set(path: &Path, config: &Config, sets: &[String]) -> Result<()> {
    let state = apply_sets(load(path, config), sets)?;
    save(path, &state)
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo fmt && cargo nextest run -E 'test(toolbar::state)'`
Expected: PASS, the 5 existing state tests and the 4 new ones.

- [ ] **Step 5: Commit**

```bash
cargo clippy --all-targets -- -D warnings
jj commit -m "toolbar: state as JSON and KEY=VALUE sets, for the Noctalia plugin" src/toolbar/state.rs
```

---

### Task 2: `valw toolbar --state | --set | --run`

**Files:**
- Modify: `src/toolbar/mod.rs` (`Picked`, `run`; new `remember`, `run_remembered`; a new `tests` module at the bottom)
- Modify: `src/main.rs` (`Command::Toolbar`, its arm in `run`, `flag_conflicts` test)

**Interfaces:**
- Consumes: Task 1's `state::{to_json, set, load, save, default_path}`, `Mode: clap::ValueEnum`.
- Produces:
  - `impl From<ToolbarState> for Picked`
  - `pub fn remember(path: &Path, config: &Config, mode: Option<Mode>) -> ToolbarState`
  - `pub fn run_remembered(config: &Config, mode: Option<Mode>) -> Result<Picked>`
  - CLI: `valw toolbar --state`, `valw toolbar --set KEY=VALUE` (repeatable), `valw toolbar --run [MODE]`, mutually exclusive.

- [ ] **Step 1: Write the failing tests**

Append to the end of `src/toolbar/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pick_carries_the_state_options() {
        let state = ToolbarState {
            mode: Mode::Window,
            timer: 5,
            cursor: true,
            preview: false,
            sound: true,
        };
        assert_eq!(
            Picked::from(state),
            Picked {
                mode: Mode::Window,
                cursor: true,
                preview: false,
                sound: true,
            }
        );
    }

    #[test]
    fn run_remembers_a_given_mode() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("toolbar.toml");
        let config = Config::default();
        state::save(&path, &ToolbarState { timer: 5, ..state::defaults(&config) }).unwrap();

        let state = remember(&path, &config, Some(Mode::Zoom));
        assert_eq!((state.mode, state.timer), (Mode::Zoom, 5), "the options stay");
        assert_eq!(state::load(&path, &config).mode, Mode::Zoom, "saved");

        assert_eq!(remember(&path, &config, None).mode, Mode::Zoom, "no mode: the remembered one");
    }

    #[test]
    fn run_without_a_mode_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("toolbar.toml");
        let config = Config::default();
        assert_eq!(remember(&path, &config, None), state::defaults(&config));
        assert!(!path.exists());
    }
}
```

In `src/main.rs`, inside `fn flag_conflicts()`, after `assert!(parses(&["toolbar"]));` add:

```rust
        assert!(parses(&["toolbar", "--state"]));
        assert!(parses(&["toolbar", "--set", "timer=5", "--set", "sound=false"]));
        assert!(parses(&["toolbar", "--run"]));
        assert!(parses(&["toolbar", "--run", "zoom"]));
        assert!(!parses(&["toolbar", "--run", "video"]));
        assert!(!parses(&["toolbar", "--state", "--run"]));
        assert!(!parses(&["toolbar", "--state", "--set", "timer=5"]));
        assert!(!parses(&["toolbar", "--set", "timer=5", "--run", "zoom"]));
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo nextest run -E 'test(toolbar::tests) | test(flag_conflicts)'`
Expected: compile errors: `Picked: From<ToolbarState>` not implemented, `remember` not found.

- [ ] **Step 3: Implement**

In `src/toolbar/mod.rs`: add `use std::path::Path;` next to the other `std` import, and after the `Picked` struct:

```rust
impl From<ToolbarState> for Picked {
    fn from(s: ToolbarState) -> Self {
        Picked {
            mode: s.mode,
            cursor: s.cursor,
            preview: s.preview,
            sound: s.sound,
        }
    }
}
```

Replace the end of `pub fn run` (from `countdown(…)?;` to the closing `}`) with:

```rust
    countdown(&mut wl, &output, picked.0.timer)?;
    Ok(Picked::from(picked.0))
}

/// The remembered state, with `mode` (if given) remembered as the new mode.
pub fn remember(path: &Path, config: &Config, mode: Option<Mode>) -> ToolbarState {
    let mut s = state::load(path, config);
    if let Some(mode) = mode {
        s.mode = mode;
        if let Err(e) = state::save(path, &s) {
            tracing::warn!("could not remember the toolbar choice: {e:#}");
        }
    }
    s
}

/// `valw toolbar --run`: the remembered options without the bar (for the
/// Noctalia plugin), then the countdown.
pub fn run_remembered(config: &Config, mode: Option<Mode>) -> Result<Picked> {
    let s = remember(&state::default_path(), config, mode);
    if s.timer > 0 {
        let mut wl = Wayland::connect()?;
        let outputs = wl.outputs();
        anyhow::ensure!(!outputs.is_empty(), "the compositor reported no outputs");
        let output = outputs[crate::focused_output(&outputs)].clone();
        countdown(&mut wl, &output, s.timer)?;
    }
    Ok(Picked::from(s))
}
```

(`pick` returns `(ToolbarState { mode, ..state }, mode)`, so `picked.0.mode == picked.1` and `Picked::from(picked.0)` is what `run` built before.)

In `src/main.rs`, replace the `Toolbar` variant:

```rust
    /// Pick a mode from a floating bar (Cmd+Shift+5).
    Toolbar {
        /// Print the remembered choices as one line of JSON (for the Noctalia plugin).
        #[arg(long, conflicts_with_all = ["set", "run"])]
        state: bool,
        /// Change a remembered choice: mode, timer, cursor, preview or sound.
        #[arg(long, value_name = "KEY=VALUE", conflicts_with = "run")]
        set: Vec<String>,
        /// Capture at once with the remembered choices, without the bar.
        #[arg(long, value_name = "MODE", num_args = 0..=1)]
        run: Option<Option<toolbar::state::Mode>>,
    },
```

and its arm in `fn run`:

```rust
        Command::Toolbar { state, set, run } => {
            let config = config::load(&config::default_path())?;
            let path = toolbar::state::default_path();
            if state {
                println!("{}", toolbar::state::to_json(&toolbar::state::load(&path, &config)));
                return Ok(());
            }
            if !set.is_empty() {
                return toolbar::state::set(&path, &config, &set);
            }
            let picked = match run {
                Some(mode) => toolbar::run_remembered(&config, mode)?,
                None => toolbar::run(&config)?,
            };
            let (mode, common) = from_toolbar(picked);
            capture(mode, common)
        }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo fmt && cargo nextest run -E 'test(toolbar) | test(flag_conflicts) | test(cli_definition)'`
Expected: PASS (including `cli_definition_is_valid`).

- [ ] **Step 5: Check the real binary's stdout (Review Focus 5)**

Run (no surfaces, safe on the live session; uses a throwaway state dir):

```bash
export XDG_STATE_HOME=$(mktemp -d)
cargo run -q -- toolbar --state | python3 -c 'import json,sys; lines=sys.stdin.read().splitlines(); assert len(lines)==1, lines; print(json.loads(lines[0]))'
cargo run -q -- toolbar --set timer=10 --set mode=zoom && cargo run -q -- toolbar --state
cargo run -q -- toolbar --set timer=7; echo "exit=$?"
cat $XDG_STATE_HOME/valw/toolbar.toml
unset XDG_STATE_HOME
```

Expected: the defaults dict printed; then a line with `"mode":"zoom"` and `"timer":10`; then an error mentioning `timer must be 0, 5 or 10` and `exit=1`; the file still says `timer = 10`.

- [ ] **Step 6: Commit**

```bash
cargo clippy --all-targets -- -D warnings
jj commit -m "toolbar: --state, --set and --run for the Noctalia plugin" src/toolbar/mod.rs src/main.rs
```

---

### Task 3: The plugin, its Luau tests, packaging and the flake check

**Files:**
- Create: `noctalia-plugin/plugin.toml`, `noctalia-plugin/bar.luau`, `noctalia-plugin/panel.luau`, `noctalia-plugin/shortcut.luau`, `noctalia-plugin/translations/en.json`, `noctalia-plugin/translations/tr.json`, `noctalia-plugin/README.md`
- Create: `noctalia-plugin/tests/stubs.luau`, `noctalia-plugin/tests/panel_test.luau`, `noctalia-plugin/tests/bar_test.luau`, `noctalia-plugin/tests/shortcut_test.luau`, `noctalia-plugin/tests/run.sh`
- Modify: `nix/package.ulu.nix` (`postInstall`)
- Modify: `nix/checks.ulu.nix` (new `noctalia-plugin` check)

**Interfaces:**
- Consumes: Task 2's CLI: `valw toolbar --state` (JSON `{mode,timer,cursor,preview,sound,timers}`), `valw toolbar --set KEY=VALUE`, `valw toolbar --run MODE`; exit 3 = cancelled.
- Produces: `$out/share/valw/noctalia-plugin/` (without `tests/`), panel id `elars/valw:toolbar`, widget type `elars/valw:bar`.

- [ ] **Step 1: Write the test harness and the failing tests**

`noctalia-plugin/tests/stubs.luau` (stands in for Noctalia's globals and records what the plugin does):

```lua
--!nonstrict
-- Stand-ins for Noctalia's globals. Each test is run as one chunk:
-- these stubs, then the entry script, then the test (see run.sh).

calls = { run = {}, notify = {}, toggled = {}, closed = 0, order = {} }
fake = {
  exists = true,
  -- runAsync answers by command prefix: { exitCode, stdout, stderr }
  answers = {},
  decoded = nil,
}
rendered = nil

local function node(kind)
  return function(props, children)
    return { kind = kind, props = props or {}, children = children or {} }
  end
end

ui = {
  row = node("row"), column = node("column"), label = node("label"),
  button = node("button"), glyph = node("glyph"), toggle = node("toggle"),
  select = node("select"), separator = node("separator"), spacer = node("spacer"),
}

noctalia = {
  tr = function(key) return key end,
  commandExists = function(_name) return fake.exists end,
  runAsync = function(command, callback)
    table.insert(calls.run, command)
    table.insert(calls.order, "run")
    if callback then
      local answer = { exitCode = 0, stdout = "", stderr = "" }
      for prefix, a in pairs(fake.answers) do
        if string.sub(command, 1, #prefix) == prefix then answer = a end
      end
      callback(answer)
    end
    return true
  end,
  notifyError = function(title, body) table.insert(calls.notify, { title, body }) end,
  togglePanel = function(id) table.insert(calls.toggled, id) end,
  json = { decode = function(_text) return fake.decoded end },
}

panel = {
  render = function(tree) rendered = tree end,
  close = function()
    calls.closed += 1
    table.insert(calls.order, "close")
  end,
}

barWidget = {
  render = function(tree) rendered = tree end,
  setTooltip = function(_text) end,
}

shortcut = {
  setLabel = function(_text) end,
  setIcon = function(_a, _b) end,
}

-- Every node of `tree` that `match` accepts, depth first.
function findAll(tree, match, out)
  out = out or {}
  if type(tree) ~= "table" then return out end
  if tree.kind and match(tree) then table.insert(out, tree) end
  for _, child in ipairs(tree.children or {}) do findAll(child, match, out) end
  return out
end

function button(text)
  local found = findAll(rendered, function(n) return n.kind == "button" and n.props.text == text end)
  assert(#found == 1, "one button " .. text .. ", found " .. #found)
  return found[1]
end

function reset()
  calls = { run = {}, notify = {}, toggled = {}, closed = 0, order = {} }
end

function ok(name)
  print("ok - " .. name)
end
```

`noctalia-plugin/tests/panel_test.luau`:

```lua
--!nonstrict
-- The toolbar panel. Runs after stubs.luau and panel.luau.

local STATE = { mode = "zoom", timer = 5, cursor = false, preview = true, sound = true, timers = { 0, 5, 10 } }

-- valw missing from Noctalia's PATH: a label and a notification, no panel.
fake.exists = false
onOpen(nil)
local labels = findAll(rendered, function(n) return n.kind == "label" end)
assert(#labels == 1 and labels[1].props.text == "panel.unavailable", "unavailable label")
assert(#calls.notify == 1, "notified once")
assert(#calls.run == 0, "nothing run")
ok("valw missing")

-- --state failing: the same.
reset()
fake.exists = true
fake.answers["valw toolbar --state"] = { exitCode = 1, stdout = "", stderr = "warn\nerror: broken\n" }
onOpen(nil)
assert(findAll(rendered, function(n) return n.kind == "label" and n.props.text == "panel.unavailable" end)[1], "label")
assert(calls.notify[1][2] == "error: broken", "the stderr's last line: " .. tostring(calls.notify[1][2]))
ok("--state failing")

-- A working valw: the remembered mode is primary, the timer selected.
reset()
fake.answers["valw toolbar --state"] = { exitCode = 0, stdout = "{}", stderr = "" }
fake.decoded = STATE
onOpen(nil)
assert(calls.run[1] == "valw toolbar --state", calls.run[1])
assert(button("mode.zoom").props.variant == "primary", "zoom is primary")
for _, m in ipairs({ "mode.screen", "mode.region", "mode.window" }) do
  assert(button(m).props.variant == "outline", m .. " is outline")
end
local select = findAll(rendered, function(n) return n.kind == "select" end)[1]
assert(select.props.selectedIndex == 1, "5 s is index 1 (0-based)")
assert(select.props.options[1] == "timer.off" and select.props.options[2] == "5 s", "timer labels")
local toggles = findAll(rendered, function(n) return n.kind == "toggle" end)
assert(#toggles == 3, "three toggles")
assert(toggles[1].props.checked == false and toggles[2].props.checked == true, "cursor off, preview on")
local hex = findAll(rendered, function(n)
  for _, v in pairs(n.props) do
    if type(v) == "string" and string.find(v, "^#") then return true end
  end
  return false
end)
assert(#hex == 0, "no hex colours")
ok("renders the state")

-- A mode click closes the panel first, then captures after the wait.
reset()
button("mode.region").props.onClick()
assert(calls.order[1] == "close" and calls.order[2] == "run", "close, then run")
assert(calls.run[1] == "sleep 0.3 && valw toolbar --run region", calls.run[1])
assert(#calls.notify == 0, "success is silent")
ok("mode click")

-- A failed capture notifies; a cancelled one doesn't.
reset()
fake.answers["sleep 0.3 && valw toolbar --run"] = { exitCode = 1, stdout = "", stderr = "error: no outputs\n" }
button("mode.screen").props.onClick()
assert(calls.notify[1][2] == "error: no outputs", "failure notified")
reset()
fake.answers["sleep 0.3 && valw toolbar --run"] = { exitCode = 3, stdout = "", stderr = "" }
button("mode.window").props.onClick()
assert(#calls.notify == 0, "cancel is silent")
ok("capture results")

-- The timer and the toggles write through --set and re-render at once.
reset()
fake.answers["sleep 0.3 && valw toolbar --run"] = nil
select = findAll(rendered, function(n) return n.kind == "select" end)[1]
select.props.onChange("2")
assert(calls.run[1] == "valw toolbar --set timer=10", calls.run[1])
select = findAll(rendered, function(n) return n.kind == "select" end)[1]
assert(select.props.selectedIndex == 2, "re-rendered with 10 s")
reset()
toggles = findAll(rendered, function(n) return n.kind == "toggle" end)
toggles[3].props.onChange("false")
assert(calls.run[1] == "valw toolbar --set sound=false", calls.run[1])
toggles = findAll(rendered, function(n) return n.kind == "toggle" end)
assert(toggles[3].props.checked == false, "re-rendered with sound off")
reset()
fake.answers["valw toolbar --set"] = { exitCode = 1, stdout = "", stderr = "error: read-only\n" }
toggles[1].props.onChange("true")
assert(calls.run[1] == "valw toolbar --set cursor=true", calls.run[1])
assert(calls.notify[1][2] == "error: read-only", "a failed --set notifies")
ok("timer and toggles")
```

`noctalia-plugin/tests/bar_test.luau`:

```lua
--!nonstrict
-- The bar widget. Runs after stubs.luau and bar.luau.

update()
assert(findAll(rendered, function(n) return n.kind == "glyph" and n.props.name == "camera" end)[1], "camera glyph")
ok("renders")

onClick()
assert(calls.toggled[1] == "elars/valw:toolbar", "left click toggles the panel")
assert(#calls.run == 0)
ok("left click")

reset()
onRightClick()
assert(calls.run[1] == "valw region", calls.run[1])
reset()
fake.answers["valw region"] = { exitCode = 1, stdout = "", stderr = "error: busy\n" }
onRightClick()
assert(calls.notify[1][2] == "error: busy", "failure notified")
reset()
fake.answers["valw region"] = { exitCode = 3, stdout = "", stderr = "" }
onRightClick()
assert(#calls.notify == 0, "cancel is silent")
ok("right click")
```

`noctalia-plugin/tests/shortcut_test.luau`:

```lua
--!nonstrict
-- The control-center tile. Runs after stubs.luau and shortcut.luau.

onClick()
assert(calls.run[1] == "noctalia msg panel-close; sleep 0.3 && valw region", calls.run[1])
reset()
fake.answers["noctalia msg panel-close"] = { exitCode = 1, stdout = "", stderr = "error: busy\n" }
onClick()
assert(calls.notify[1][2] == "error: busy", "failure notified")
reset()
fake.answers["noctalia msg panel-close"] = { exitCode = 3, stdout = "", stderr = "" }
onClick()
assert(#calls.notify == 0, "cancel is silent")
ok("click")

reset()
onRightClick()
assert(calls.toggled[1] == "elars/valw:toolbar", "right click toggles the panel")
ok("right click")
```

`noctalia-plugin/tests/run.sh`:

```sh
#!/bin/sh
# Runs each plugin test as one chunk: the stubs, the entry script, the test.
# Needs `luau` (nix shell nixpkgs#luau -c sh noctalia-plugin/tests/run.sh).
set -eu
dir=$(dirname "$0")
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
for pair in panel:panel_test bar:bar_test shortcut:shortcut_test; do
  entry=${pair%%:*}
  test=${pair##*:}
  cat "$dir/stubs.luau" "$dir/../$entry.luau" "$dir/$test.luau" > "$tmp/$test.luau"
  luau "$tmp/$test.luau"
done
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `nix shell nixpkgs#luau -c sh noctalia-plugin/tests/run.sh; echo "exit=$?"`
Expected: `cat` fails on the missing `panel.luau` (`No such file or directory`), `exit=1`.

- [ ] **Step 3: Write the plugin**

`noctalia-plugin/plugin.toml`:

```toml
# valw for Noctalia: a bar button, the toolbar as a Noctalia panel and a
# control-center tile. Thin clients of the valw CLI: the choices live in
# valw's own toolbar.toml (`valw toolbar --state` / `--set`), captures run
# through `valw toolbar --run` and `valw region`.

id = "elars/valw"
name = "valw"
version = "0.1.0"
plugin_api = 3
author = "elars"
license = "MIT"
dependencies = ["valw"]
tags = ["screenshot", "capture"]
icon = "camera"
description = "macOS-style screenshots with valw: a bar button, a toolbar panel and a control-center tile."

# Left click: the toolbar panel. Right click: a region shot.
[[widget]]
id = "bar"
entry = "bar.luau"

[[panel]]
id = "toolbar"
entry = "panel.luau"
width = 360
height = 220
placement = "attached"
position = "auto"
open_near_click = true

# Click: a region shot. Right click: the toolbar panel.
[[shortcut]]
id = "tile"
entry = "shortcut.luau"
```

`noctalia-plugin/bar.luau`:

```lua
--!nonstrict
-- valw bar widget: a camera glyph. Left click opens the toolbar panel,
-- right click takes a region shot at once (plain `valw region`, like the
-- keybind: the toolbar's timer and options don't apply).

local PANEL = "elars/valw:toolbar"

local function lastLine(text)
  local last = ""
  for line in string.gmatch(text or "", "[^\n]+") do
    last = line
  end
  return last
end

-- Exit 3 is a cancel (Esc): not an error.
local function report(result)
  if result.exitCode ~= 0 and result.exitCode ~= 3 then
    noctalia.notifyError("valw", lastLine(result.stderr))
  end
end

function update()
  barWidget.render(ui.row({ align = "center" }, {
    ui.glyph({ name = "camera" }),
  }))
  barWidget.setTooltip(noctalia.tr("bar.tooltip"))
end

function onClick()
  noctalia.togglePanel(PANEL)
end

function onRightClick()
  noctalia.runAsync("valw region", report)
end
```

`noctalia-plugin/panel.luau`:

```lua
--!nonstrict
-- valw toolbar panel: the four modes, the timer and the options, drawn
-- with Noctalia's own components. The choices live in valw's toolbar.toml:
-- read with `valw toolbar --state`, changed with `valw toolbar --set`.
-- A mode click closes the panel, waits for it to go, then captures.

local MODES = {
  { id = "screen", glyph = "device-desktop" },
  { id = "region", glyph = "crop" },
  { id = "window", glyph = "app-window" },
  { id = "zoom", glyph = "zoom-in" },
}
local OPTIONS = { "cursor", "preview", "sound" }

-- The decoded `--state` JSON; nil until it arrives.
local state = nil
local unavailable = false

local function lastLine(text)
  local last = ""
  for line in string.gmatch(text or "", "[^\n]+") do
    last = line
  end
  return last
end

-- Exit 3 is a cancel (Esc): not an error.
local function report(result)
  if result.exitCode ~= 0 and result.exitCode ~= 3 then
    noctalia.notifyError("valw", lastLine(result.stderr))
  end
end

local render

local function capture(mode)
  panel.close()
  noctalia.runAsync("sleep 0.3 && valw toolbar --run " .. mode, report)
end

-- `key` and `value` are always one of our own tokens, never user text.
local function remember(key, value)
  state[key] = value
  render()
  noctalia.runAsync("valw toolbar --set " .. key .. "=" .. tostring(value), report)
end

local function modeButton(m)
  return ui.button({
    text = noctalia.tr("mode." .. m.id),
    glyph = m.glyph,
    variant = if state.mode == m.id then "primary" else "outline",
    onClick = function()
      capture(m.id)
    end,
  })
end

local function optionRow(key)
  return ui.row({ gap = 8, align = "center", justify = "space_between" }, {
    ui.label({ text = noctalia.tr("option." .. key), color = "on_surface" }),
    ui.toggle({
      checked = state[key],
      onChange = function(value)
        remember(key, value == "true")
      end,
    }),
  })
end

render = function()
  if unavailable then
    panel.render(ui.column({ padding = 16 }, {
      ui.label({ text = noctalia.tr("panel.unavailable"), color = "error" }),
    }))
    return
  end
  if state == nil then
    return
  end
  local modes = {}
  for _, m in ipairs(MODES) do
    table.insert(modes, modeButton(m))
  end
  local timers, selected = {}, 0
  for i, t in ipairs(state.timers) do
    table.insert(timers, if t == 0 then noctalia.tr("timer.off") else tostring(t) .. " s")
    if t == state.timer then
      selected = i - 1
    end
  end
  local rows = {
    ui.row({ gap = 8, justify = "space_between" }, modes),
    ui.separator({}),
    ui.row({ gap = 8, align = "center", justify = "space_between" }, {
      ui.label({ text = noctalia.tr("timer.label"), color = "on_surface" }),
      ui.select({
        options = timers,
        selectedIndex = selected,
        onChange = function(index)
          remember("timer", state.timers[tonumber(index) + 1])
        end,
      }),
    }),
  }
  for _, key in ipairs(OPTIONS) do
    table.insert(rows, optionRow(key))
  end
  panel.render(ui.column({ gap = 12, padding = 16 }, rows))
end

local function fail(message)
  unavailable = true
  noctalia.notifyError("valw", message)
  render()
end

function onOpen(_context)
  unavailable = false
  if not noctalia.commandExists("valw") then
    fail(noctalia.tr("panel.unavailable"))
    return
  end
  noctalia.runAsync("valw toolbar --state", function(result)
    if result.exitCode ~= 0 then
      fail(lastLine(result.stderr))
      return
    end
    local decoded = noctalia.json.decode(result.stdout)
    if type(decoded) ~= "table" then
      fail(noctalia.tr("panel.unavailable"))
      return
    end
    state = decoded
    render()
  end)
end
```

`noctalia-plugin/shortcut.luau`:

```lua
--!nonstrict
-- valw control-center tile. Click: the control center closes, then a
-- region shot (so it isn't in it). Right click: the toolbar panel.

local PANEL = "elars/valw:toolbar"

local function lastLine(text)
  local last = ""
  for line in string.gmatch(text or "", "[^\n]+") do
    last = line
  end
  return last
end

-- Exit 3 is a cancel (Esc): not an error.
local function report(result)
  if result.exitCode ~= 0 and result.exitCode ~= 3 then
    noctalia.notifyError("valw", lastLine(result.stderr))
  end
end

shortcut.setLabel(noctalia.tr("tile.label"))
shortcut.setIcon("camera", "camera")

function onClick()
  noctalia.runAsync("noctalia msg panel-close; sleep 0.3 && valw region", report)
end

function onRightClick()
  noctalia.togglePanel(PANEL)
end
```

`noctalia-plugin/translations/en.json`:

```json
{
  "bar": { "tooltip": "Screenshot" },
  "tile": { "label": "Screenshot" },
  "panel": { "unavailable": "valw is not available" },
  "mode": { "screen": "Screen", "region": "Region", "window": "Window", "zoom": "Zoom" },
  "timer": { "label": "Timer", "off": "Off" },
  "option": { "cursor": "Show cursor", "preview": "Show preview", "sound": "Play sound" }
}
```

`noctalia-plugin/translations/tr.json`:

```json
{
  "bar": { "tooltip": "Ekran görüntüsü" },
  "tile": { "label": "Ekran görüntüsü" },
  "panel": { "unavailable": "valw bulunamadı" },
  "mode": { "screen": "Ekran", "region": "Bölge", "window": "Pencere", "zoom": "Yakınlaştır" },
  "timer": { "label": "Zamanlayıcı", "off": "Kapalı" },
  "option": { "cursor": "İmleci göster", "preview": "Önizlemeyi göster", "sound": "Ses çal" }
}
```

`noctalia-plugin/README.md`:

```markdown
# valw for Noctalia

A bar button, the valw toolbar as a Noctalia panel, and a control-center
tile. Noctalia draws everything, so it follows your Noctalia theme.

- Bar button: left click opens the toolbar, right click takes a region shot.
- Toolbar: click a mode to capture; the timer and options are remembered
  (shared with `valw toolbar`).
- Tile: click for a region shot, right click for the toolbar.

Install it with the home-manager module (`programs.valw.noctalia.enable`),
then enable `elars/valw` in Noctalia and place the widget (type
`elars/valw:bar`). Toggle the toolbar from a keybind with
`noctalia msg panel-toggle elars/valw:toolbar`.

Tests: `nix shell nixpkgs#luau -c sh noctalia-plugin/tests/run.sh`.
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `nix shell nixpkgs#luau -c sh -c 'sh noctalia-plugin/tests/run.sh && luau-compile --null noctalia-plugin/*.luau && echo ALL-OK'`
Expected: `ok - …` lines for every test (6 panel, 3 bar, 2 tile), then the compile summary and `ALL-OK`. A failing `assert` would print the message and stop with a non-zero exit (check that once: temporarily change `"valw region"` in `bar_test.luau` to `"valw regio"`, see it fail, revert).

- [ ] **Step 5: Package it and add the flake check**

In `nix/package.ulu.nix`, append to `postInstall` (after the `install -Dm644 …LICENSE-DejaVu…` line):

```nix
          # The Noctalia plugin (programs.valw.noctalia), without its tests.
          mkdir -p $out/share/valw
          cp -r noctalia-plugin $out/share/valw/noctalia-plugin
          rm -r $out/share/valw/noctalia-plugin/tests
```

In `nix/checks.ulu.nix`, add to `checks` (after `home-module-rejects`):

```nix
        # The plugin as packaged: manifest, every entry present, Luau that
        # compiles, translations that parse; and its logic against stubbed
        # Noctalia globals.
        noctalia-plugin =
          pkgs.runCommand "valw-noctalia-plugin"
            {
              nativeBuildInputs = [
                pkgs.jq
                pkgs.luau
              ];
            }
            ''
              dir=${valw}/share/valw/noctalia-plugin
              grep -qx 'id = "elars/valw"' $dir/plugin.toml
              test "$(grep -c '^entry = ' $dir/plugin.toml)" -eq 3
              for entry in $(sed -n 's/^entry = "\(.*\)"$/\1/p' $dir/plugin.toml); do
                test -f $dir/$entry
              done
              test ! -e $dir/tests
              luau-compile --null $dir/*.luau
              for f in $dir/translations/*.json; do jq empty $f; done
              cp -r ${../noctalia-plugin} plugin
              chmod -R u+w plugin
              sh plugin/tests/run.sh
              touch $out
            '';
```

- [ ] **Step 6: Run the check**

Run: `nix build .#checks.x86_64-linux.noctalia-plugin -L 2>&1 | tail -20`
Expected: the `ok - …` lines in the log and a successful build.

- [ ] **Step 7: Commit**

```bash
jj commit -m "noctalia-plugin: bar widget, toolbar panel and tile, with Luau tests and a flake check" noctalia-plugin nix/package.ulu.nix nix/checks.ulu.nix
```

---

### Task 4: home-manager option, README and checklist

**Files:**
- Modify: `nix/home-module.ulu.nix` (option `noctalia.enable`, `xdg.dataFile`)
- Modify: `nix/checks.ulu.nix` (`homeModule` helper takes the whole `programs.valw` config; `xdg.dataFile` stand-in; assertions)
- Modify: `README.md` (a "Noctalia" section; `valw toolbar` flags in the commands table)
- Modify: `docs/test-checklist.md` (a "Noctalia plugin" section)

**Interfaces:**
- Consumes: Task 3's `$out/share/valw/noctalia-plugin`.
- Produces: `programs.valw.noctalia.enable` → `xdg.dataFile."noctalia/plugins/valw".source = "${cfg.package}/share/valw/noctalia-plugin"`.

- [ ] **Step 1: Write the failing check**

In `nix/checks.ulu.nix`, change the helper's signature and body so callers pass the whole `programs.valw` config:

```nix
      homeModule =
        valwConfig:
        (lib.evalModules {
          modules = [
            self.homeModules.default
            (
              { lib, ... }:
              let
                files = lib.mkOption {
                  type = lib.types.attrsOf (
                    lib.types.submodule { options.source = lib.mkOption { type = lib.types.path; }; }
                  );
                  default = { };
                };
              in
              {
                options.home.packages = lib.mkOption {
                  type = lib.types.listOf lib.types.package;
                  default = [ ];
                };
                options.xdg.configFile = files;
                options.xdg.dataFile = files;
                config._module.args.pkgs = pkgs;
                config.programs.valw = {
                  enable = true;
                }
                // valwConfig;
              }
            )
          ];
        }).config;
```

Update the callers: in `home-module`, `plain = homeModule { };`, `set = homeModule { settings = { preview.timeout_secs = 3; sound.volume = 0.4; }; };`, and add `noctalia = homeModule { noctalia.enable = true; };` to its `let`; in `home-module-rejects`, `fails = settings: pkgs.testers.testBuildFailure (homeModule { inherit settings; }).xdg.configFile."valw/config.toml".source;`.

Add these asserts to `home-module`, next to the existing ones:

```nix
          assert lib.assertMsg (plain.xdg.dataFile == { }) "no Noctalia plugin unless asked";
          assert lib.assertMsg (
            noctalia.xdg.dataFile."noctalia/plugins/valw".source == "${valw}/share/valw/noctalia-plugin"
          ) "the plugin is linked where Noctalia looks";
```

- [ ] **Step 2: Run it to verify it fails**

Run: `nix build .#checks.x86_64-linux.home-module 2>&1 | tail -5`
Expected: evaluation error: `The option 'programs.valw.noctalia' does not exist.`

- [ ] **Step 3: Implement the option**

In `nix/home-module.ulu.nix`, add to `options.programs.valw` (after `settings`):

```nix
        noctalia.enable = lib.mkEnableOption ''
          valw's Noctalia plugin (a bar button, the toolbar as a Noctalia
          panel and a control-center tile), linked into
          `$XDG_DATA_HOME/noctalia/plugins/valw`. Enable `elars/valw` and
          place its widget in Noctalia's own settings
        '';
```

and to `config = lib.mkIf cfg.enable { … }`:

```nix
        xdg.dataFile."noctalia/plugins/valw" = lib.mkIf cfg.noctalia.enable {
          source = "${cfg.package}/share/valw/noctalia-plugin";
        };
```

- [ ] **Step 4: Run the checks to verify they pass**

Run: `nix build .#checks.x86_64-linux.home-module .#checks.x86_64-linux.home-module-rejects 2>&1 | tail -5`
Expected: both build.

- [ ] **Step 5: README and checklist**

In `README.md`, in the commands table change the toolbar row to:

```markdown
| `valw toolbar` | Pick a mode from a floating bar, with a timer and options; `--state`, `--set KEY=VALUE` and `--run [MODE]` drive it without the bar (the Noctalia plugin uses them) |
```

and add after the "The overview backdrop" section:

````markdown
## Noctalia

With [Noctalia](https://noctalia.dev), valw comes as a plugin: a camera
button on the bar (left click: the toolbar, right click: a region shot),
the toolbar as a Noctalia panel and a control-center tile. Noctalia draws
them, so they follow your Noctalia theme.

```nix
programs.valw.noctalia.enable = true;
```

Then enable `elars/valw` in Noctalia, add a widget of type `elars/valw:bar`
to a bar, and bind the toolbar:

```kdl
Mod+Shift+T { spawn "noctalia" "msg" "panel-toggle" "elars/valw:toolbar"; }
```
````

In `docs/test-checklist.md`, add a section at the end:

```markdown
## Noctalia plugin

- [ ] `programs.valw.noctalia.enable` + `elars/valw` enabled: the camera button appears where the widget is placed; its tooltip says Screenshot.
- [ ] Left click opens the toolbar panel next to the bar; it uses Noctalia's colours, font, corners and border; switching the palette or light/dark restyles it.
- [ ] The remembered mode is highlighted; Screen / Region / Window / Zoom each capture, and the panel is never in the shot.
- [ ] Timer and the three toggles persist, and match what `valw toolbar` (the self-drawn bar) shows, both ways; the timer shows valw's countdown.
- [ ] Right click on the button: a region shot (no timer).
- [ ] Control-center tile: click closes the control center, then a region shot; right click opens the toolbar panel.
- [ ] `noctalia msg panel-toggle elars/valw:toolbar` (Mod+Shift+T) opens and closes the panel.
- [ ] With valw off Noctalia's PATH: "valw is not available" and a notification, no crash.
- [ ] Turkish locale: the Turkish strings appear.
```

- [ ] **Step 6: Commit**

```bash
jj commit -m "home-manager: programs.valw.noctalia.enable; README and checklist" nix/home-module.ulu.nix nix/checks.ulu.nix README.md docs/test-checklist.md
```

---

### Task 5: Load it in the live Noctalia (verify the folder name and the symlink)

**Files:**
- Possibly modify: `nix/home-module.ulu.nix`, `nix/checks.ulu.nix`, `README.md`, `noctalia-plugin/README.md` (only if the folder name or the symlink needs a change)

**Interfaces:**
- Consumes: Task 3's plugin folder; Task 4's install path `noctalia/plugins/valw`.
- Produces: a verified install path (ledgered as a ruling if it differs).

- [ ] **Step 1: Tell the user** that the plugin is about to be loaded into their running Noctalia (a symlink in `~/.local/share/noctalia/plugins/`, `noctalia msg plugins enable elars/valw`, the panel opened once), and wait for their go-ahead.

- [ ] **Step 2: Link the built plugin the way home-manager will (a symlink into the store)**

```bash
out=$(nix build --no-link --print-out-paths .#default)
mkdir -p ~/.local/share/noctalia/plugins
test ! -e ~/.local/share/noctalia/plugins/valw
ln -s $out/share/valw/noctalia-plugin ~/.local/share/noctalia/plugins/valw
noctalia msg config-reload
noctalia msg plugins list | grep -i valw
```

Expected: a line `elars/valw [...] 0.1.0 disabled` (or similar). If it is missing: check `~/.cache/noctalia/noctalia.log` for the plugin loader's messages, try the folder names `elars-valw` and `elars/valw` (nested), and if a store symlink is refused, a plain copy; ledger the result as `Task 5: Ruling: …` and apply it to `nix/home-module.ulu.nix` (`recursive = true` for a copy, or the new folder name), the `home-module` check's expected key, and both READMEs.

- [ ] **Step 3: Enable it and open the panel once**

```bash
noctalia msg plugins enable elars/valw
noctalia msg panel-toggle elars/valw:toolbar
```

Expected: the panel appears near the bar with the four modes, the timer and three toggles, in Noctalia's style. Ask the user to confirm what they see, then close it (`noctalia msg panel-close`). With the user's go-ahead, also take two real shots from the panel: one with the timer at 5 s (the countdown runs, the shot lands, no error notification) and one Region shot held open for more than 6 s before selecting (it must not be killed). Check that nothing is clipped at 360×220 (the "Play sound" row, the four mode buttons in Turkish); adjust `width`/`height` in `plugin.toml` if needed. Check `grep -i valw ~/.cache/noctalia/noctalia.log | tail -20` for Luau errors (unknown glyph names, bad props); fix any in the plugin, rerun Task 3's tests, and commit (`jj commit -m "noctalia-plugin: fixes from the live Noctalia" noctalia-plugin`).

- [ ] **Step 4: Undo the dev install**

```bash
noctalia msg plugins disable elars/valw
rm ~/.local/share/noctalia/plugins/valw
```

(home-manager will own that path after the user's rebuild; a leftover link would collide.)

---

### Task 6: copland wiring (after the final review; needs the user's approval)

**Files (in `~/copland`, only after the user approves the diff):**
- Modify: `modules/home/home-valw.ulu.nix` (`programs.valw.noctalia.enable = true;`; Mod+Shift+T → `noctalia msg panel-toggle elars/valw:toolbar`)
- Modify: `modules/home/home-noctalia.ulu.nix` (`"elars/valw"` in `plugins.enabled`; `widget.valw = { type = "elars/valw:bar"; };`; `"valw"` in the bar's `end` list or a capsule group, where the user wants it)
- Modify: `flake.lock` (`nix flake update valw`)

- [ ] **Step 1:** Push valw `main` first (after `nix flake check` passes), then show the user the copland diff and ask where the button goes on the bar.
- [ ] **Step 2:** On approval: edit, `nix flake update valw`, `jj commit -m "valw: Noctalia plugin" <the three paths>` (never `modules/nixos/common/common-packages.ulu.nix`); the user rebuilds.
