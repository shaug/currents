import CurrentsPlatform
import SwiftUI

// Synthetic iOS availability fixture only. No acquisition or entitlement request.
@main struct CaptureFixture: App {
    var body: some Scene {
        WindowGroup {
            Text("Synthetic fixture. Automatic acquisition is blocked.")
                .task {
                    _ = await CaptureAvailability().automaticAcquisition()
                }
        }
    }
}
