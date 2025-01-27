{ pkgs, lib, config, inputs, ... }:

{
  # https://devenv.sh/basics/
  env.GREET = "devenv";

  # https://devenv.sh/packages/
  packages = [ pkgs.git pkgs.cargo-flamegraph ];

  # https://devenv.sh/languages/
  languages.rust = {
    enable = true;
    # https://devenv.sh/reference/options/#languagesrustchannel
    channel = "nightly";
    components = [ "rustc" "cargo" "clippy" "rustfmt" "rust-analyzer"  ];
  };

  # https://devenv.sh/processes/
  # processes.cargo-watch.exec = "cargo-watch";

  # https://devenv.sh/services/

  # https://devenv.sh/scripts/
  # scripts.hello.exec = ''
  #   echo hello from $GREET
  # '';

  # enterShell = ''
  #   hello
  #   git --version
  # '';

  # https://devenv.sh/tasks/
  # tasks = {
  #   "myproj:setup".exec = "mytool build";
  #   "devenv:enterShell".after = [ "myproj:setup" ];
  # };

  # https://devenv.sh/tests/
  enterTest = ''
    echo "Running tests"
    cargo test
  '';

  # https://devenv.sh/pre-commit-hooks/
  git-hooks.hooks = {
      
     # lint shell scripts
     shellcheck.enable = true;
     # execute example shell from Markdown files
     mdsh.enable = true;
     cargo-check.enable = true;
     end-of-file-fixer.enable = true;
     clippy.enable = true;
     clippy.packageOverrides.cargo = pkgs.cargo;
     clippy.packageOverrides.clippy = pkgs.clippy;
  }; 

  # See full reference at https://devenv.sh/reference/options/
}
