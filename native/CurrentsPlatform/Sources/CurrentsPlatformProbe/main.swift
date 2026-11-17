import CurrentsPlatform

// Framework-independent CLT diagnostic. This is not an app bundle launch or a
// substitute for CurrentsPlatformTests in the full Xcode qualification matrix.
@main struct SyntheticProbe {
    static func main() async {
        let probe = PlatformProbeActor()
        for value: UInt64 in [0, 1, UInt64.max] {
            guard await probe.roundtrip(value) == value else {
                fatalError("synthetic_actor_c_roundtrip failed")
            }
        }
        print("test synthetic_actor_c_roundtrip ... ok")
        let capture = CaptureAvailability()
        guard await capture.automaticAcquisition() == .blocked else {
            fatalError("synthetic_missing_entitlement_refuses_acquisition failed")
        }
        print("test synthetic_missing_entitlement_refuses_acquisition ... ok")
        print("Synthetic probe only. Native platform qualification remains BLOCKED.")
    }
}
