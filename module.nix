{ config, pkgs, lib, ... }:

let
  cfg = config.services.evremap;
  evremapPkg = import ./default.nix { inherit pkgs };

  tomlFormat = pkgs.formats.toml { };
  evremapConfig = tomlFormat.generate "evremap.toml" cfg.settings;
in 

with lib;

{
  options = {
    services.evremap = {
      enable = mkOption {
        default = false;
        type = with types; bool;
        description = ''
          Enable evremap service
        '';
      };
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
        ExecStart = "${evremapPkg}/bin/evremap remap ${evremapConfig}";
        Restart = "always";
      };
    };

    environment.systemPackages = [evremapPkg];
  };
}
