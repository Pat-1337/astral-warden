{
  lib,
  rustPlatform,
  makeWrapper,
  addDriverRunpath,
  libnotify,
  util-linux,
}:

let
  root = ../..;
  manifest = (lib.importTOML (root + "/Cargo.toml")).package;
in
rustPlatform.buildRustPackage {
  pname = manifest.name;
  inherit (manifest) version;

  src = lib.fileset.toSource {
    inherit root;
    fileset = lib.fileset.unions [
      (root + "/Cargo.toml")
      (root + "/Cargo.lock")
      (root + "/build.rs")
      (root + "/src")
      (root + "/assets")
      (root + "/include")
      (root + "/init")
    ];
  };

  cargoLock.lockFile = root + "/Cargo.lock";

  nativeBuildInputs = [ makeWrapper ];

  postInstall = ''
    install -Dm644 init/udev.rules $out/lib/udev/rules.d/70-gpu-warden.rules
    install -Dm644 init/modules-load.conf $out/lib/modules-load.d/gpu-warden.conf
    install -Dm644 include/gpu_warden.h $out/include/gpu_warden.h
  '';

  postFixup = ''
    wrapProgram $out/bin/gpu-warden \
      --prefix PATH : ${
        lib.makeBinPath [
          libnotify
          util-linux
        ]
      } \
      --suffix LD_LIBRARY_PATH : ${addDriverRunpath.driverLink}/lib
  '';

  meta = {
    inherit (manifest) description;
    homepage = "https://github.com/patvuk/gpu-warden";
    license = lib.licenses.mit;
    platforms = lib.platforms.linux;
    mainProgram = "gpu-warden";
  };
}
