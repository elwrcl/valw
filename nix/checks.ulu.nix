{ self, ... }:
{
  perSystem =
    {
      pkgs,
      lib,
      self',
      ...
    }:
    let
      valw = self'.packages.default;

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

      # Reuses the package's vendored dependencies, runs `cmd` instead of the build.
      cargoCheck =
        name: tools: cmd:
        valw.overrideAttrs (old: {
          pname = "valw-${name}";
          nativeBuildInputs = old.nativeBuildInputs ++ tools;
          buildPhase = cmd;
          installPhase = "touch $out";
          # The package patches its binary's RUNPATH; a check has no binary.
          postFixup = "";
        });
    in
    {
      checks = {
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
            # Quiet, and no home directory needed.
            HOME=/nonexistent ${valw}/bin/valw __check-config ${
              set.xdg.configFile."valw/config.toml".source
            } 2> err
            test ! -s err
            touch $out
          '';

        home-module-rejects =
          let
            fails =
              settings:
              pkgs.testers.testBuildFailure (homeModule settings).xdg.configFile."valw/config.toml".source;
          in
          pkgs.runCommand "valw-home-module-rejects" { } ''
            grep -q 'sound.volume must be between 0 and 1' ${fails { sound.volume = 2; }}/testBuildFailure.log
            grep -q 'unknown config key: preview.timout_secs' ${
              fails { preview.timout_secs = 3; }
            }/testBuildFailure.log
            touch $out
          '';
        clippy = cargoCheck "clippy" [ pkgs.clippy ] "cargo clippy --all-targets --offline -- -D warnings";
        nextest = cargoCheck "nextest" [ pkgs.cargo-nextest ] "cargo nextest run --offline";

        fmt = pkgs.runCommand "valw-fmt" { nativeBuildInputs = [ pkgs.rustfmt ]; } ''
          rustfmt --check --edition 2024 ${valw.src}/src/main.rs
          touch $out
        '';

        # Runs the real screencopy path against a headless sway. This is
        # wlroots, not niri, but it catches capture and format bugs on every build.
        headless =
          pkgs.runCommand "valw-headless"
            {
              nativeBuildInputs = [
                pkgs.file
                pkgs.grim
                pkgs.imagemagick
                pkgs.sway-unwrapped
                valw
              ];
            }
            ''
              export HOME=$TMPDIR/home XDG_RUNTIME_DIR=$TMPDIR/run
              mkdir -p $HOME
              mkdir -m 700 $XDG_RUNTIME_DIR
              export WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1

              echo 'output HEADLESS-1 resolution 1280x720' > sway.conf
              sway -c sway.conf > sway.log 2>&1 &

              for _ in $(seq 100); do
                socket=$(ls $XDG_RUNTIME_DIR | grep -m1 '^wayland-[0-9]*$' || true)
                [ -n "$socket" ] && break
                sleep 0.1
              done
              [ -n "$socket" ] || { cat sway.log; exit 1; }
              export WAYLAND_DISPLAY=$socket

              valw doctor | tee doctor.txt
              ! grep -q '^fail' doctor.txt

              # Through a pipe: the clipboard child must not keep it open.
              # The timeout covers the reader too; the child is in its own
              # session, so bounding valw alone would not catch a leak.
              timeout 10 sh -c 'valw screen -o - | cat > shot.png'
              file shot.png | tee file.txt
              grep -q 'PNG image data, 1280 x 720' file.txt

              # Preview. The background is solid black, so a thumbnail in the
              # bottom-right corner shows up as more than one colour there.
              mkdir -p $HOME/.config/valw
              printf '[preview]\ntimeout_secs = 2\n' > $HOME/.config/valw/config.toml
              socket=$XDG_RUNTIME_DIR/valw.sock
              corner() { magick "$1" -crop 300x200+980+520 +repage -format '%k' info:; }
              wait_for() { # $1: test expression, $2: tenths of a second
                for _ in $(seq "$2"); do eval "$1" && return 0; sleep 0.1; done
                echo "timed out waiting for: $1"
                exit 1
              }

              valw screen > /dev/null
              wait_for '[ -S $socket ]' 10
              sleep 0.5
              grim with-thumbnail.png
              [ "$(corner with-thumbnail.png)" -gt 1 ] || { echo "thumbnail not visible"; exit 1; }
              valw screen --no-preview -o hidden.png > /dev/null
              [ "$(corner hidden.png)" -eq 1 ] || { echo "thumbnail ended up in a screenshot"; exit 1; }
              wait_for '[ ! -S $socket ]' 40

              valw screen --no-preview > /dev/null
              sleep 1
              if [ -S $socket ]; then
                echo "--no-preview started a host"
                exit 1
              fi

              touch $out
            '';
      };
    };
}
