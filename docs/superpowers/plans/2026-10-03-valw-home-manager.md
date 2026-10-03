# valw home-manager module Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `flake.homeModules.default` with `programs.valw.{enable, package, settings}`, the config checked at build time by `valw __check-config`, plus a README.

**Architecture:** A strict `config::check` behind a hidden CLI command; a dendritic `nix/home-module.ulu.nix` that writes `settings` with `pkgs.formats.toml` through a checking `runCommand`; two flake checks that evaluate the module with stub home-manager options (no new input).

**Tech Stack:** Rust 2024; Nix (flake-parts, dendritic `*.ulu.nix`), `pkgs.formats.toml`, `pkgs.testers.testBuildFailure`.

**Spec:** `docs/superpowers/specs/2026-10-03-valw-home-manager-design.md`

## Global Constraints

- jj only; `cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`, `cargo nextest run`, `nix flake check`.
- No manual `imports = [ ... ]` in the dendritic flake; no new flake inputs.
- Error text for unknown keys: `unknown config key: a` / `unknown config keys: a, b`.

## Review Focus

1. **The checked config is byte-identical to what valw reads** (no re-serialisation in between). Task 2's check greps the installed file.
2. **`__check-config` in the sandbox** (no HOME, no Wayland, no niri) must work. Task 2's checks run it there.
3. **Empty `settings` writes no file** (valw defaults apply). Task 2 asserts it.
4. **A typo'd key fails the build with the key's name.** Task 2's rejects check.
5. **Toolbar state stays writable.** Not managed (spec), README says so.

---

### Task 1: `valw __check-config`

**Files:** `src/config.rs`, `src/main.rs`.

- [ ] **Step 1: Failing tests**

`config.rs` tests:

```rust
    #[test]
    fn check_accepts_valid_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[preview]\ntimeout_secs = 3\n[sound]\nvolume = 0.4\n").unwrap();
        check(&path).unwrap();
        std::fs::write(&path, "").unwrap();
        check(&path).unwrap();
    }

    #[test]
    fn check_rejects_invalid_values_with_valws_message() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[sound]\nvolume = 2.0\n").unwrap();
        let err = format!("{:#}", check(&path).unwrap_err());
        assert!(err.contains("sound.volume must be between 0 and 1"), "{err}");
    }

    #[test]
    fn check_rejects_unknown_keys_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[preview]\ntimout_secs = 3\n").unwrap();
        assert_eq!(check(&path).unwrap_err().to_string(), "unknown config key: preview.timout_secs");
        std::fs::write(&path, "[preview]\ntimout_secs = 3\n[sund]\nx = 1\n").unwrap();
        assert_eq!(
            check(&path).unwrap_err().to_string(),
            "unknown config keys: preview.timout_secs, sund"
        );
    }
```

`main.rs`: `flag_conflicts` gains `assert!(parses(&["__check-config", "c.toml"]));` and `assert!(!parses(&["__check-config"]));`; the help test gains `assert!(!help.contains("check-config"), "{help}");`.

- [ ] **Step 2: Implement**

`config.rs`:

```rust
/// Checks a config file strictly, for home-manager's build: invalid values
/// and unknown keys are errors (at run time unknown keys only warn).
pub fn check(path: &Path) -> Result<()> {
    let text = std::fs::read_to_string(path).with_context(|| format!("could not read {}", path.display()))?;
    let (_, unknown) = parse(&text).with_context(|| format!("invalid config file {}", path.display()))?;
    match unknown.len() {
        0 => Ok(()),
        1 => bail!("unknown config key: {}", unknown[0]),
        _ => bail!("unknown config keys: {}", unknown.join(", ")),
    }
}
```

`main.rs`: `/// Internal: check a config file (home-manager runs it at build time).` `#[command(name = "__check-config", hide = true)] CheckConfig { file: PathBuf }` → `Command::CheckConfig { file } => config::check(&file),`.

- [ ] **Step 3: Run** (+3 tests), clippy, fmt. **Step 4: Commit** `feat: valw __check-config`.

---

### Task 2: The module and its checks

**Files:** create `nix/home-module.ulu.nix`; modify `nix/checks.ulu.nix`.

- [ ] **Step 1: Failing checks** — `nix/checks.ulu.nix`: take `self` and `lib` in the module arguments, and add

```nix
      # The home-manager module, evaluated with stand-ins for the two
      # home-manager options it sets (no home-manager input needed).
      homeModule =
        settings:
        (lib.evalModules {
          modules = [
            self.homeModules.default
            (
              { lib, ... }:
              {
                options.home.packages = lib.mkOption {
                  type = lib.types.listOf lib.types.package;
                  default = [ ];
                };
                options.xdg.configFile = lib.mkOption {
                  type = lib.types.attrsOf (
                    lib.types.submodule { options.source = lib.mkOption { type = lib.types.path; }; }
                  );
                  default = { };
                };
                config._module.args.pkgs = pkgs;
                config.programs.valw = {
                  enable = true;
                  inherit settings;
                };
              }
            )
          ];
        }).config;
```

(in the `let`), and in `checks`:

```nix
        home-module =
          let
            plain = homeModule { };
            set = homeModule {
              preview.timeout_secs = 3;
              sound.volume = 0.4;
            };
          in
          assert lib.assertMsg (plain.home.packages == [ valw ]) "the module installs valw";
          assert lib.assertMsg (plain.xdg.configFile == { }) "no settings, no config file";
          pkgs.runCommand "valw-home-module" { } ''
            grep -qx 'timeout_secs = 3' ${set.xdg.configFile."valw/config.toml".source}
            grep -qx 'volume = 0.4' ${set.xdg.configFile."valw/config.toml".source}
            touch $out
          '';

        home-module-rejects =
          let
            fails = settings: pkgs.testers.testBuildFailure (homeModule settings).xdg.configFile."valw/config.toml".source;
          in
          pkgs.runCommand "valw-home-module-rejects" { } ''
            grep -q 'sound.volume must be between 0 and 1' ${fails { sound.volume = 2; }}/testBuildFailure.log
            grep -q 'unknown config key: preview.timout_secs' ${fails { preview.timout_secs = 3; }}/testBuildFailure.log
            touch $out
          '';
```

Run `nix flake check path:. --no-build` (evaluation only) → fails: `self.homeModules` missing.

- [ ] **Step 2: The module** — `nix/home-module.ulu.nix`:

```nix
{ withSystem, ... }:
{
  flake.homeModules.default =
    {
      config,
      lib,
      pkgs,
      ...
    }:
    let
      cfg = config.programs.valw;
      toml = pkgs.formats.toml { };
      # valw checks its own config: a typo or a bad value fails the build.
      checked =
        settings:
        let
          file = toml.generate "valw-config.toml" settings;
        in
        pkgs.runCommand "valw-config.toml" { } ''
          ${lib.getExe cfg.package} __check-config ${file}
          cp ${file} $out
        '';
    in
    {
      options.programs.valw = {
        enable = lib.mkEnableOption "valw, macOS-style screenshots for niri";
        package = lib.mkOption {
          type = lib.types.package;
          default = withSystem pkgs.stdenv.hostPlatform.system ({ config, ... }: config.packages.default);
          defaultText = lib.literalExpression "valw.packages.\${system}.default";
          description = "The valw package to install and to check the settings with.";
        };
        settings = lib.mkOption {
          inherit (toml) type;
          default = { };
          example = lib.literalExpression ''
            {
              preview.timeout_secs = 3;
              sound.volume = 0.4;
              capture.window_shadow = true;
            }
          '';
          description = ''
            valw's config (`$XDG_CONFIG_HOME/valw/config.toml`), checked by
            valw at build time. Empty means valw's defaults.
          '';
        };
      };

      config = lib.mkIf cfg.enable {
        home.packages = [ cfg.package ];
        xdg.configFile."valw/config.toml" = lib.mkIf (cfg.settings != { }) {
          source = checked cfg.settings;
        };
      };
    };
}
```

- [ ] **Step 3: Run** — `nix flake check path:. -L --keep-going`: all pass, including `home-module` and `home-module-rejects`.

- [ ] **Step 4: Commit** `feat: home-manager module`.

---

### Task 3: README and checklist

- [ ] `README.md`: what valw is (one paragraph); install (`nix run`, flake package, the module); the module example (from the spec) and a note that the toolbar's memory stays in `~/.local/state/valw`; niri bind examples (`Print` → `valw screen`, `Mod+Shift+S` → `valw region`, plus `window`, `zoom`, `toolbar`); a table of the commands; `valw doctor`; where logs are.
- [ ] Checklist section:

```markdown

## home-manager module

- [ ] copland: `home-valw.ulu.nix` uses `inputs.valw.homeModules.default` with `programs.valw.enable = true` and a few `settings`; rebuild; `~/.config/valw/config.toml` is the store file with those settings.
- [ ] A typo in `settings` (e.g. `preview.timout_secs`) fails the rebuild naming the key.
- [ ] The toolbar still remembers its options (state file stays writable).
- [ ] New binds for `valw window`, `valw zoom`, `valw toolbar` in copland's niri keybinds.
```

- [ ] Commit `docs: README and home-manager checklist`.
