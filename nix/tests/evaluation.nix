{
  self,
  nixpkgs,
  home-manager,
  pkgs,
}:
let
  inherit (pkgs) lib;
  package = pkgs.writeShellScriptBin "lg-buddy" "exit 0";
  systemWith =
    module: extra:
    nixpkgs.lib.nixosSystem {
      inherit (pkgs.stdenv.hostPlatform) system;
      modules = [
        module
        {
          system.stateVersion = "26.05";
          boot.loader.grub.enable = false;
          fileSystems."/" = {
            device = "/dev/test-root";
            fsType = "ext4";
          };
          networking.networkmanager.enable = true;
          users.users.alice = {
            isNormalUser = true;
            home = "/home/alice";
          };
          services.lg-buddy = {
            enable = true;
            user = "alice";
            inherit package;
          };
        }
        extra
      ];
    };
  system = systemWith self.nixosModules.lg-buddy;
  homeWithModule =
    module: extraSpecialArgs: extra:
    home-manager.lib.homeManagerConfiguration {
      inherit pkgs extraSpecialArgs;
      modules = [
        module
        {
          home = {
            username = "alice";
            homeDirectory = "/home/alice";
            stateVersion = "26.05";
          };
          services.lg-buddy = {
            enable = true;
            inherit package;
          };
        }
        extra
      ];
    };
  homeWith = homeWithModule self.homeManagerModules.lg-buddy;
  home = homeWith { };
  integrated =
    overrides:
    homeWith {
      osConfig.services.lg-buddy = {
        enable = true;
        user = "alice";
        configPath = "/home/alice/.config/lg-buddy/config.env";
        inherit package;
      }
      // overrides;
    } { };
  machine = (system { }).config;
  user = (home { }).config;
  disabledMachine = (system { services.lg-buddy.enable = lib.mkForce false; }).config;
  disabledHome = (home { services.lg-buddy.enable = lib.mkForce false; }).config;
  fails = value: !(builtins.tryEval (builtins.deepSeq value true)).success;
  check = condition: message: if condition then true else throw message;
  checks = [
    (check (builtins.all
      (
        channel:
        let
          machinePackage =
            (systemWith self.nixosModules.${channel} { }).options.services.lg-buddy.package.default;
          homePackage =
            (homeWithModule self.homeManagerModules.${channel} { } { })
            .options.services.lg-buddy.package.default;
          version =
            (builtins.fromTOML (builtins.readFile "${self}/crates/lg-buddy/Cargo.toml")).package.version;
          isPrerelease = lib.hasInfix "-" version;
          expected =
            if channel == "dev" then
              "dev"
            else if isPrerelease then
              "prerelease"
            else
              "stable";
        in
        if channel == "stable" && isPrerelease then
          fails machinePackage.drvPath && fails homePackage.drvPath
        else
          machinePackage.outPath == homePackage.outPath
          && machinePackage.releaseChannel == expected
          &&
            machinePackage.outPath == self.packages.${pkgs.stdenv.hostPlatform.system}.${
              if
                builtins.elem channel [
                  "default"
                  "lg-buddy"
                ]
              then
                channel
              else
                "lg-buddy-${channel}"
            }.outPath
      )
      [
        "default"
        "lg-buddy"
        "dev"
        "stable"
        "prerelease"
      ]
    ) "Channel modules and package outputs must select the same build identity")
    (check (
      let
        release = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
        factoryDefaults = [
          (pkgs.callPackage ../package.nix {
            src = self;
            buildCommit = self.rev or self.dirtyRev or "unknown";
          })
          (systemWith (import ../modules/nixos.nix { source = self; }) { })
          .options.services.lg-buddy.package.default
          (homeWithModule (import ../modules/home-manager.nix { source = self; }) { } { })
          .options.services.lg-buddy.package.default
        ];
      in
      builtins.all (package: package.outPath == release.outPath) factoryDefaults
    ) "Package and module factories must default to release identity")
    (check (
      machine.services.lg-buddy.configPath == user.services.lg-buddy.configPath
    ) "Default machine/user config must agree")
    (check (
      !machine.systemd.services.LG_Buddy.restartIfChanged
      && !machine.systemd.services.LG_Buddy.stopIfChanged
    ) "Rebuilds must not power-cycle the TV")
    (check (
      !(machine.systemd.user.services ? LG_Buddy_screen)
    ) "Machine module must not install global user units")
    (check (
      !(disabledMachine.systemd.services ? LG_Buddy)
    ) "Disabled machine module must have no services")
    (check (
      !(disabledHome.systemd.user.services ? LG_Buddy_screen)
    ) "Disabled home module must have no monitor")
    (check (
      !(user.systemd.user.services.LG_Buddy_screen.Unit ? ConditionPathExists)
    ) "First pairing needs a running monitor before config exists")
    (check (
      user.systemd.user.services.LG_Buddy_screen.Unit.X-SwitchMethod == "keep-old"
    ) "Refresh owns existing monitor transitions")
    (check (fails
      (home { services.lg-buddy.settings.screen.idle_timeout = 0; }).config.services.lg-buddy.settings
    ) "Invalid timeout must fail")
    (check (fails
      (home { services.lg-buddy.settings.tv.ip = "127.0.0.1"; }).config.services.lg-buddy.settings
    ) "TV profiles must not enter store declarations")
    (check (fails
      (home { services.lg-buddy.configPath = "/nix/store/config.env"; })
      .config.services.lg-buddy.configPath
    ) "Mutable config must not enter store")
    (check (fails
      (home { services.lg-buddy.configPath = "/home/alice/../config.env"; })
      .config.services.lg-buddy.configPath
    ) "Noncanonical config must fail")
    (check (builtins.all (
      a: a.assertion
    ) machine.assertions) "Valid machine configuration must pass assertions")
    (check (builtins.all (a: a.assertion)
      (integrated { }).config.assertions
    ) "Matching integrated home must pass assertions")
    (check (builtins.all (overrides: fails (integrated overrides).activationPackage) [
      { user = "bob"; }
      { configPath = "/home/alice/other.env"; }
      { package = pkgs.writeShellScriptBin "lg-buddy" "exit 1"; }
    ]) "Mismatched user, config or package must fail")
    (check (builtins.any (a: !a.assertion)
      (system { networking.networkmanager.enable = lib.mkForce false; }).config.assertions
    ) "Missing NetworkManager must fail")
  ];
in
assert builtins.all (x: x) checks;
pkgs.runCommand "lg-buddy-module-evaluation" { } ''touch "$out"''
