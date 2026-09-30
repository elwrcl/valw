{ ... }:
{
  perSystem =
    { pkgs, self', ... }:
    let
      valw = self'.packages.default;

      # Reuses the package's vendored dependencies, runs `cmd` instead of the build.
      cargoCheck =
        name: tools: cmd:
        valw.overrideAttrs (old: {
          pname = "valw-${name}";
          nativeBuildInputs = old.nativeBuildInputs ++ tools;
          buildPhase = cmd;
          installPhase = "touch $out";
        });
    in
    {
      checks = {
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
