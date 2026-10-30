{
  self,
  home-manager,
  pkgs,
}:
let
  inherit (pkgs) lib;
  # Exercise both Exec escaping layers, including literal desktop field codes.
  homeDirectory = "/build/LG Buddy \"home\" $`%";
  peer =
    revision:
    let
      runtime = pkgs.writeShellScript "lg-buddy" ''
        ${pkgs.coreutils}/bin/readlink -f "$0" >> "$HOME/launches"
      '';
    in
    pkgs.runCommand "lg-buddy-launcher-${revision}" { meta.mainProgram = "lg-buddy"; } ''
      install -Dm755 ${runtime} "$out/bin/lg-buddy"
      install -Dm644 ${../../io.github.staphylococcus.LGBuddy.desktop} \
        "$out/share/applications/io.github.staphylococcus.LGBuddy.desktop"
      substituteInPlace "$out/share/applications/io.github.staphylococcus.LGBuddy.desktop" \
        --replace-fail 'Exec=/usr/bin/lg-buddy' "Exec=$out/bin/lg-buddy"
    '';
  first = lib.hiPrio (peer "first");
  second = peer "second";
  home =
    package: useXdg:
    (home-manager.lib.homeManagerConfiguration {
      inherit pkgs;
      modules = [
        self.homeManagerModules.lg-buddy
        {
          home = {
            username = "alice";
            inherit homeDirectory;
            stateVersion = "26.05";
          };
          nix.assumeXdg = useXdg;
          services.lg-buddy = {
            enable = true;
            inherit package;
          };
        }
      ];
    }).config;
  homes =
    map
      (useXdg: {
        first = home first useXdg;
        second = home second useXdg;
      })
      [
        false
        true
      ];
  python = pkgs.python3.withPackages (p: [ p.pygobject3 ]);
  decoy = pkgs.writeShellScriptBin "lg-buddy" "exit 77";
in
pkgs.runCommand "lg-buddy-cached-home-launcher" { } ''
  export HOME=${lib.escapeShellArg homeDirectory}
  export PATH=${decoy}/bin
  export GI_TYPELIB_PATH=${pkgs.glib}/lib/girepository-1.0
  ${python}/bin/python ${./launcher.py} ${
    lib.escapeShellArgs (
      lib.concatMap (pair: [
        pair.first.home.profileDirectory
        (toString pair.first.home.path)
        (toString pair.second.home.path)
      ]) homes
    )
  } ${first}/bin/lg-buddy ${second}/bin/lg-buddy
  ${pkgs.coreutils}/bin/touch "$out"
''
