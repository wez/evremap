{ config, pkgs, ... }:

let
  evremapPkg = import ../default.nix { inherit pkgs; };
in 
{
  config = {
    nixpkgs.overlays = [
      (final: prev: {
        evremap = evremapPkg;
      })
    ];
  };
}
