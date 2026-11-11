{
  description = "Android device body for GUI agents";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      nixpkgs,
      flake-utils,
      rust-overlay,
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
        in
        {
          devShells.default = pkgs.mkShell {
            packages = [
              pkgs.jdk21
              pkgs.gradle
              android.androidsdk
              rust
              pkgs.just
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
