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

        # Clicking a preview opens Satty. --suffix keeps a user's own Satty first.
        postInstall = ''
          wrapProgram $out/bin/valw --suffix PATH : ${lib.makeBinPath [ pkgs.satty ]}
        '';

        meta = {
          description = "macOS-style screenshots for niri";
          mainProgram = "valw";
          platforms = lib.platforms.linux;
        };
      };
    };
}
