{
  description = "rmcl: A fully featured Minecraft TUI launcher";

  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = {
    nixpkgs,
    flake-utils,
    ...
  }:
    flake-utils.lib.eachDefaultSystem (system: let
      pkgs = nixpkgs.legacyPackages.${system};
    in {
      packages.default = pkgs.rustPlatform.buildRustPackage {
        pname = "rmcl";
        version = (builtins.fromTOML (builtins.readFile ./Cargo.toml)).package.version;
        src = pkgs.lib.cleanSource ./.;
        cargoLock.lockFile = ./Cargo.lock;
        nativeBuildInputs = [pkgs.jdk];

        meta = with pkgs.lib; {
          description = "A fully featured Minecraft TUI launcher";
          homepage = "https://github.com/objz/rmcl";
          license = licenses.gpl3Only;
          mainProgram = "rmcl";
        };
      };
    });
}
