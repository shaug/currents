import Testing
@testable import CurrentsPlatform

@Test func cur_004_actor_c_roundtrip() async {
    let probe = PlatformProbeActor()
    for value: UInt64 in [0, 1, UInt64.max] {
        #expect(await probe.roundtrip(value) == value)
    }
}

@Test func cur_004_missing_entitlement_refuses_acquisition() async {
    let capture = CaptureAvailability()
    #expect(await capture.automaticAcquisition() == .blocked)
}
