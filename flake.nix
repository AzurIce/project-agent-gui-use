{
  description = "Android device body for GUI agents";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    notist = {
      # 固定到含 crates/notist-observe/src/timings.html 的版本：更新的 rev 里该文件
      # 不在 git 树中，Nix 构建会因 include_str! 失败。
      url = "github:AzurIce/notist/3d17013b60a01a7e53e68bd076540494f4c5549a";
      inputs = {
        nixpkgs.follows = "nixpkgs";
        flake-utils.follows = "flake-utils";
        rust-overlay.follows = "rust-overlay";
      };
    };
  };

  outputs =
    {
      nixpkgs,
      flake-utils,
      rust-overlay,
      notist,
      ...
    }:
    flake-utils.lib.eachSystem
      [
        "x86_64-linux"
        "aarch64-linux"
        "aarch64-darwin"
      ]
      (
        system:
        let
          pkgs = import nixpkgs {
            inherit system;
            overlays = [ (import rust-overlay) ];
            config = {
              allowUnfree = true;
              android_sdk.accept_license = true;
            };
          };

          android = pkgs.androidenv.composeAndroidPackages {
            platformVersions = [ "36" ];
            buildToolsVersions = [ "36.0.0" ];
            includeEmulator = false;
            includeNDK = false;
            includeSystemImages = false;
            includeSources = false;
          };

          rust = pkgs.rust-bin.stable.latest.default.override {
            extensions = [
              "rust-src"
              "rustfmt"
              "clippy"
            ];
          };

          notistPkg = notist.packages.${system}.default;

        in
        {
          devShells.default = pkgs.mkShell {
            packages = [
              pkgs.jdk21
              pkgs.gradle
              android.androidsdk
              rust
              pkgs.just
              notistPkg
              pkgs.python3
            ]
            ++ pkgs.lib.optionals pkgs.stdenv.hostPlatform.isLinux [
              # 调试用的桌面端镜像/控制工具，手机上不需要。
              pkgs.scrcpy
            ];

            shellHook = ''
              export JAVA_HOME="${pkgs.jdk21}"
              export ANDROID_HOME="${android.androidsdk}/libexec/android-sdk"
              export ANDROID_SDK_ROOT="$ANDROID_HOME"

              echo "gui-use dev shell"
              echo "  JAVA_HOME    $JAVA_HOME"
              echo "  ANDROID_HOME $ANDROID_HOME"
              echo "  adb          $(command -v adb || echo missing)"
            '';
          };
        }
      );
}
