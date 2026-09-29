{ ... }:
{
  perSystem =
    { pkgs, ... }:
    {
      formatter = pkgs.writeShellApplication {
        name = "nixfmt-tree";
        runtimeInputs = [
          pkgs.nixfmt
          pkgs.fd
          pkgs.findutils
        ];
        text = ''
          if [ "$#" -eq 0 ]; then
            set -- .
          fi
          fd --type f --extension nix --hidden --exclude .jj . "$@" \
            | xargs -r nixfmt
        '';
      };
    };
}
