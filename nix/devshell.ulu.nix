{ ... }:
{
  perSystem =
    { pkgs, lib, self', ... }:
    {
      devShells.default = pkgs.mkShell {
        inputsFrom = [ self'.packages.default ];

        packages = with pkgs; [
          cargo-nextest
          clippy
          rust-analyzer
          rustfmt
          # Manual verification: grim is the pixel oracle, magick compares.
          grim
          imagemagick
          wayland-utils
        ];

        # What the package gets through its RUNPATH, for `cargo run`.
        LD_LIBRARY_PATH = lib.makeLibraryPath [
          pkgs.wayland
          pkgs.libxkbcommon
          pkgs.libglvnd
        ];

        # The repo is a path: flake, so Nix copies the whole tree on every
        # evaluation. Keep the multi-gigabyte target dir out of it.
        shellHook = ''
          export CARGO_TARGET_DIR="''${XDG_CACHE_HOME:-$HOME/.cache}/valw/target"
        '';
      };
    };
}
