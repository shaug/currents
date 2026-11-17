#if os(iOS) && canImport(FinanceKit)
import FinanceKit
#endif

/// Acquisition is disabled until a separate authorized entitlement and account
/// qualification. A synthetic fixture must never initiate FinanceKit access.
@available(iOS 18.0, macOS 15.0, *)
public actor CaptureAvailability {
    public enum Outcome: Sendable { case blocked }
    public init() {}
    public func automaticAcquisition() -> Outcome { .blocked }
}
