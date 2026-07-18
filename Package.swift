// swift-tools-version:5.9
import PackageDescription

let package = Package(
    name: "OSCARKit",
    platforms: [
        .iOS(.v16),
        .macOS(.v13),
    ],
    products: [
        .library(name: "OSCARKit", targets: ["OSCARKit"]),
    ],
    dependencies: [
        // Only needed for Insecure.MD5 — Apple platforms could alternatively use
        // CommonCrypto directly, but swift-crypto's API is much less painful.
        .package(url: "https://github.com/apple/swift-crypto.git", from: "3.0.0"),
    ],
    targets: [
        .target(
            name: "OSCARKit",
            dependencies: [
                .product(name: "Crypto", package: "swift-crypto"),
            ]
        ),
    ]
)
