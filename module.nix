{ config, pkgs, lib, ... }:

let
  cfg = config.services.evremap;
  evremapPkg = import ./default.nix { inherit pkgs; };

  tomlFormat = pkgs.formats.toml { };
  evremapConfig = tomlFormat.generate "evremap.toml" cfg.settings;
in 

with lib;

{
  nixpkgs.config.packageOverrides = pkgs: {
    evremap = evremapPkg;
  };

  options = {
    services.evremap = {
      enable = mkEnableOption "evremap";

      package = mkPackageOption pkgs ["evremap" "evremap"] { };

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
        ExecStart = "${pkgs.evremap.evremap}/bin/evremap remap ${evremapConfig}";
        Restart = "always";
      };
    };

    environment.systemPackages = [evremapPkg];
  };
}
