{ ... }:
{
  perSystem =
    { pkgs, lib, ... }:
    {
      packages.default = pkgs.rustPlatform.buildRustPackage {
        pname = "valw";
        version = "0.1.0";

        src = lib.cleanSourceWith {
          src = ../.;
          filter =
            path: _type:
            !(builtins.elem (baseNameOf path) [
              ".direnv"
              ".jj"
              # Plan ledgers and review notes; not part of the build.
              ".superpowers"
              "result"
              "target"
            ]);
        };

        cargoLock.lockFile = ../Cargo.lock;

        nativeBuildInputs = [
          pkgs.makeWrapper
          pkgs.pkg-config
        ];
        buildInputs = [
          pkgs.libxkbcommon
          pkgs.wayland
        ];

        # Tests run through `nix flake check` instead.
        doCheck = false;

        # Clicking a preview opens Satty if configured. --suffix keeps a user's own Satty first.
        # The editor embeds DejaVu Sans Bold; its licence wants the notice shipped.
        postInstall = ''
          wrapProgram $out/bin/valw --suffix PATH : ${
            lib.makeBinPath [
              pkgs.satty
              pkgs.pipewire
            ]
          }
          install -Dm644 assets/fonts/LICENSE-DejaVu $out/share/licenses/valw/LICENSE-DejaVu
        '';

        # dlopen()ed at run time: libwayland (winit, valw), libxkbcommon (winit),
        # libEGL (zoom, editor; Mesa's drivers come from the system).
        postFixup = ''
          patchelf --add-rpath ${
            lib.makeLibraryPath [
              pkgs.wayland
              pkgs.libxkbcommon
              pkgs.libglvnd
            ]
          } $out/bin/.valw-wrapped
        '';

        meta = {
          description = "macOS-style screenshots for niri";
          mainProgram = "valw";
          platforms = lib.platforms.linux;
        };
      };
    };
}
