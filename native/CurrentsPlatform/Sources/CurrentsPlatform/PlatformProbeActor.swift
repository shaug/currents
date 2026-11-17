import PlatformProbe

/// A synthetic actor/C import probe. Does not implement or substitute for C14.
public actor PlatformProbeActor {
    public init() {}
    public func roundtrip(_ value: UInt64) -> UInt64 {
        currents_platform_probe_roundtrip(value)
    }
}
