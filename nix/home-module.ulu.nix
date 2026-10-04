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
        noctalia.enable = lib.mkEnableOption ''
          valw's Noctalia plugin (a bar button, the toolbar as a Noctalia
          panel and a control-center tile), linked into
          `$XDG_DATA_HOME/noctalia/plugins/valw`. Enable `elars/valw` and
          place its widget in Noctalia's own settings
        '';
      };

      config = lib.mkIf cfg.enable {
        home.packages = [ cfg.package ];
        xdg.configFile."valw/config.toml" = lib.mkIf (cfg.settings != { }) {
          source = checked cfg.settings;
        };
        xdg.dataFile."noctalia/plugins/valw" = lib.mkIf cfg.noctalia.enable {
          source = "${cfg.package}/share/valw/noctalia-plugin";
        };
      };
    };
}
