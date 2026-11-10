import CurrentsPlatform
import Foundation
import Testing

@Test func cur_004_actor_c_roundtrip() async {
    let probe = PlatformProbeActor()
    for value: UInt64 in [0, 1, UInt64.max] {
        #expect(await probe.roundtrip(value) == value)
    }
}

@Test func cur_004_4() async {
    #expect(await CaptureAvailability().automaticAcquisition() == .blocked)
}

#if os(macOS)
@Test func cur_004_1() throws {
    let products = try #require(ProcessInfo.processInfo.environment["CURRENTS_BUILT_PRODUCTS_DIR"])
    for name in ["CurrentsApp", "CurrentsOwner"] {
        let bundleURL = URL(fileURLWithPath: products).appendingPathComponent("\(name).app")
        let bundle = try #require(Bundle(url: bundleURL))
        let executable = try #require(bundle.executableURL)
        #expect(bundle.object(forInfoDictionaryKey: "LSMinimumSystemVersion") as? String == "15.0")
        let child = Process()
        let output = Pipe()
        child.executableURL = executable
        child.standardOutput = output
        try child.run()
        let bytes = output.fileHandleForReading.readDataToEndOfFile()
        child.waitUntilExit()
        #expect(child.terminationStatus == 0)
        #expect(String(decoding: bytes, as: UTF8.self).contains("synthetic_actor_c_roundtrip ... ok"))
        #expect(String(decoding: bytes, as: UTF8.self).contains("Native platform qualification remains BLOCKED"))
    }
}
#endif
