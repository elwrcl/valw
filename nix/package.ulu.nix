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
              ".superpowers"
              "docs"
              "README.md"
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

        doCheck = false;
        postInstall = ''
          wrapProgram $out/bin/valw --suffix PATH : ${
            lib.makeBinPath [
              pkgs.satty
              pkgs.pipewire
            ]
          }
          install -Dm644 assets/fonts/LICENSE-DejaVu $out/share/licenses/valw/LICENSE-DejaVu
          mkdir -p $out/share/valw
          cp -r noctalia-plugin $out/share/valw/noctalia-plugin
          rm -r $out/share/valw/noctalia-plugin/tests
        '';

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
          description = "desktop helper";
          mainProgram = "valw";
          platforms = lib.platforms.linux;
        };
      };
    };
}
