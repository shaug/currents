// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "CurrentsPlatform",
    platforms: [.macOS(.v15), .iOS(.v18)],
    products: [
        .library(name: "CurrentsPlatform", targets: ["CurrentsPlatform"]),
        .executable(name: "currents-platform-probe", targets: ["CurrentsPlatformProbe"])
    ],
    targets: [
        // This private C probe is not the production C14 ABI.
        .target(name: "PlatformProbe", publicHeadersPath: "include"),
        .target(name: "CurrentsPlatform", dependencies: ["PlatformProbe"]),
        .executableTarget(name: "CurrentsPlatformProbe", dependencies: ["CurrentsPlatform"]),
        .testTarget(name: "CurrentsPlatformTests", dependencies: ["CurrentsPlatform"])
    ],
    swiftLanguageModes: [.v6]
)
