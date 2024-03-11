{ config, pkgs, lib, ... }:

let
  cfg = config.services.evremap;

  tomlFormat = pkgs.formats.toml { };
  evremapConfig = tomlFormat.generate "evremap.toml" cfg.settings;
in 

with lib;

{
  imports = [
    ./overlays.nix
  ];

  options = {
    services.evremap = {
      enable = mkEnableOption "evremap";

      package = mkPackageOption pkgs "evremap" { };

      settings = mkOption {
        default = {};
        type = with types; attrs;
        description = ''
          Evremap settings converted to TOML file
        '';
      };
    };
  };

  config = mkIf cfg.enable {
    systemd.services.evremap = {
      wantedBy = [ "multi-user.target" ];
      serviceConfig = {
        WorkingDirectory = "/";
        ExecStart = "${cfg.package}/bin/evremap remap ${evremapConfig}";
        Restart = "always";
      };
    };

    environment.systemPackages = [cfg.package];
  };
}
