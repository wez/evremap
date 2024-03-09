{ config, pkgs, lib, ... }:

let
  ev = config.services.evremap;
  evremapPkg = import ./default.nix {};

  tomlFormat = pkgs.formats.toml { };
  evremapConfig = tomlFormat.generate "evremap.toml" ev.settings;
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
        type = with types; set;
        description = ''
          Evremap settings
        '';
      };
    };
  };

  config = mkIf ev.enable {
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
