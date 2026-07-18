import SwiftUI
import OSCARKit

/// Bare-bones example of wiring OSCARClient into SwiftUI. Not meant to be the
/// final UI — just proof that the plumbing works end to end before you build
/// out the retro-buddy-list aesthetic on top.
struct ChatDemoView: View {
    @StateObject private var client = OSCARClient(
        host: "your.server.host",   // matches OSCAR_ADVERTISED_LISTENERS_PLAIN
        screenName: "MyScreenName",
        password: "whatever"        // Open OSCAR Server auto-creates accounts by default
    )
    @State private var recipient = ""
    @State private var draft = ""

    var body: some View {
        VStack(spacing: 12) {
            statusView

            List(client.incomingMessages) { im in
                VStack(alignment: .leading) {
                    Text(im.from).font(.caption).foregroundStyle(.secondary)
                    Text(im.text)
                }
            }

            HStack {
                TextField("Screen name to message", text: $recipient)
                    .textFieldStyle(.roundedBorder)
            }
            HStack {
                TextField("Message", text: $draft)
                    .textFieldStyle(.roundedBorder)
                Button("Send") {
                    client.sendMessage(to: recipient, text: draft)
                    draft = ""
                }
                .disabled(recipient.isEmpty || draft.isEmpty)
            }
        }
        .padding()
        .task {
            client.login()
        }
    }

    @ViewBuilder
    private var statusView: some View {
        switch client.state {
        case .disconnected:
            Label("Disconnected", systemImage: "circle")
        case .connectingAuth, .awaitingAuthKey, .awaitingLoginResponse, .connectingBOS:
            Label("Signing in…", systemImage: "arrow.triangle.2.circlepath")
        case .online:
            Label("Online", systemImage: "checkmark.circle.fill").foregroundStyle(.green)
        case .failed(let error):
            Label("Failed: \(error.localizedDescription)", systemImage: "xmark.circle.fill")
                .foregroundStyle(.red)
        }
    }
}
