{ ... }:
{
  perSystem =
    {
      pkgs,
      lib,
      self',
      ...
    }:
    {
      devShells.default = pkgs.mkShell {
        inputsFrom = [ self'.packages.default ];

        packages = with pkgs; [
          cargo-nextest
          clippy
          rust-analyzer
          rustfmt
          grim
          imagemagick
          wayland-utils
        ];

        LD_LIBRARY_PATH = lib.makeLibraryPath [
          pkgs.wayland
          pkgs.libxkbcommon
          pkgs.libglvnd
        ];

        shellHook = ''
          export CARGO_TARGET_DIR="''${XDG_CACHE_HOME:-$HOME/.cache}/valw/target"
        '';
      };
    };
}
