Pod::Spec.new do |s|
  s.name           = 'LxmfReactNative'
  s.version        = '0.1.0'
  s.summary        = 'LXMF Reticulum mesh networking for React Native'
  s.homepage       = 'https://github.com/anon0mesh/lxmf_react_native_rust'
  s.license        = { type: 'MIT' }
  s.author         = { 'anon0mesh' => 'anon0mesh@example.com' }
  s.source         = { git: 'https://github.com/anon0mesh/lxmf_react_native_rust.git' }

  s.platform       = :ios, '13.0'
  s.requires_arc   = true
  s.swift_version  = '5.5'

  s.source_files   = 'ios/**/*.swift'
  s.public_header_files = 'ios/**/*.h'

  # Link the Rust static library (XCFramework built by scripts/build-rust-ios.sh)
  s.vendored_frameworks = 'ios/RustCore/liblxmf_rn.xcframework'
  s.libraries      = 'c++'

  s.dependency 'ExpoModulesCore'

  # BLE framework
  s.frameworks = 'CoreBluetooth', 'Foundation'
end
