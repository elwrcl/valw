# valw home-manager module (design)

Date: 2026-10-03
Status: approved in brainstorming, pending written-spec review
Builds on: every phase on `main`; the config format in `src/config.rs`

## 1. Goal

Manage valw from home-manager: install it and write its config from Nix,
with mistakes in the settings caught when the configuration is built, not
when valw runs.

### Out of scope

- A NixOS module (valw is per user; `packages.default` installs it).
- niri key bindings (they stay in the user's niri config, i.e. copland).
- The toolbar's remembered choices (`~/.local/state/valw/toolbar.toml`),
  which stay writable state.

## 2. Usage

```nix
programs.valw = {
  enable = true;
  settings = {
    preview.timeout_secs = 3;
    sound.volume = 0.4;
    capture.window_shadow = true;
  };
};
```

## 3. Options

| Option | Type | Default | Meaning |
|---|---|---|---|
| `programs.valw.enable` | bool | `false` | Install valw (`home.packages`) |
| `programs.valw.package` | package | this flake's `packages.${system}.default` | The valw to install and to check the settings with |
| `programs.valw.settings` | `(pkgs.formats.toml { }).type` | `{ }` | Written to `$XDG_CONFIG_HOME/valw/config.toml`; nothing is written when empty, so valw uses its defaults |

## 4. Checking at build time

- `settings` becomes a TOML file with `pkgs.formats.toml`.
- The file installed as `xdg.configFile."valw/config.toml".source` is a
  derivation that runs `${package}/bin/valw __check-config <file>` and
  copies the file to `$out` only if the check passes.
- `valw __check-config <file>` (hidden): parses the file exactly as valw
  does at run time. An invalid value prints valw's own error (for example
  `sound.volume must be between 0 and 1`) and exits non-zero. An unknown
  key, which valw only warns about at run time, is an error here, listing
  every unknown key (`unknown config key: preview.timout_secs`). A valid
  file prints nothing and exits 0. It needs no Wayland, niri or home
  directory.

## 5. Structure

| File | Change |
|---|---|
| `src/main.rs` | hidden `Command::CheckConfig { file }` |
| `src/config.rs` | `check(path) -> Result<()>`: parse + unknown keys as an error |
| `nix/home-module.ulu.nix` (new) | `flake.homeModules.default` |
| `nix/checks.ulu.nix` | `home-module` (evaluates the module with stub `home.packages` / `xdg.configFile` options and builds the checked config) and `home-module-rejects` (a bad setting fails to build, via `pkgs.testers.testBuildFailure`) |
| `README.md` (new) | what valw is, install (flake package or the module), the module example, niri bind examples, the commands |

No new flake input: the checks evaluate the module with `lib.evalModules`
and stubs instead of pulling in home-manager.

## 6. Testing

- Rust (`cargo nextest`): `check` accepts a valid file and the defaults,
  rejects an invalid value with valw's message, rejects unknown keys and
  names them; the CLI parses `__check-config FILE` and hides it from help.
- Nix (`nix flake check`): the two module checks above.

Manual: wiring it into copland happens in the end-of-project test pass
(checklist section).

## 7. Definition of done

`cargo clippy -- -D warnings`, `cargo fmt --check`, `cargo nextest run`
and `nix flake check` pass; README written; checklist section added.
