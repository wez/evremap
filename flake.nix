{
  description = "A flake for evremap";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
  let
    forAllSystems = nixpkgs.lib.genAttrs nixpkgs.lib.systems.flakeExposed;
  in
  {
    packages = forAllSystems (system:
    let
      pkgs = import nixpkgs { inherit system; };
      evremap = import ./default.nix { inherit pkgs; };
    in
    {
      inherit evremap;
      default = evremap;
    });
  };
}
