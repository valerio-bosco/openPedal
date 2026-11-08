{
  description = "openPedal - effetti per chitarra in tempo reale";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = nixpkgs.legacyPackages.${system};
    in
    {
      devShells.${system}.default = pkgs.mkShell {
        packages = with pkgs; [
          cargo
          rustc
          clippy
          rustfmt
          rust-analyzer
          pkg-config
          qpwgraph # per vedere i collegamenti audio
          nodejs # solo per sviluppare l'interfaccia (ui/); chi usa openPedal non ne ha bisogno
        ];
        # libjack2 serve solo per compilare (header e jack.pc); a runtime vince
        # la libjack di PipeWire grazie a LD_LIBRARY_PATH, come fa `pw-jack`.
        buildInputs = [ pkgs.libjack2 ];

        # openPedal parla JACK: la libjack di PipeWire lo collega al grafo PipeWire.
        # (Il buffer si sceglie in openpedal.toml, non qui.)
        LD_LIBRARY_PATH = "${pkgs.pipewire.jack}/lib";
      };
    };
}
