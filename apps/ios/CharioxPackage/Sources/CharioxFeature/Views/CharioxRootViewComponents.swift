import SwiftUI

struct TerminalField: View {
    let title: String
    @Binding var text: String
    let prompt: String

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            Text(title.uppercased())
                .font(.system(.caption, design: .monospaced, weight: .bold))
                .foregroundStyle(CharioxPalette.orange)
                .frame(width: 86, alignment: .leading)
            TextField(prompt, text: $text)
                .charioxTerminalInput()
                .font(.system(.caption, design: .monospaced))
                .foregroundStyle(.white)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 11)
        .background(CharioxPalette.field)
        .accessibilityLabel(title)
    }
}

struct TranscriptRow: View {
    let entry: TranscriptEntry

    var body: some View {
        VStack(alignment: .leading, spacing: 5) {
            HStack(spacing: 8) {
                Text(entry.kind.rawValue.uppercased())
                    .font(.system(.caption2, design: .monospaced, weight: .bold))
                    .foregroundStyle(color)
                if let agentID = entry.agentID {
                    Text(agentID)
                        .font(.system(.caption2, design: .monospaced))
                        .foregroundStyle(CharioxPalette.muted)
                }
                Spacer(minLength: 0)
            }
            Text(entry.text)
                .font(.system(.caption, design: .monospaced))
                .foregroundStyle(.white)
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(.vertical, 2)
    }

    private var color: Color {
        switch entry.kind {
        case .error:
            CharioxPalette.red
        case .notice, .completion, .status:
            CharioxPalette.muted
        default:
            CharioxPalette.orange
        }
    }
}

struct SummaryLine: View {
    let label: String
    let value: String

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            Text(label.uppercased())
                .font(.system(.caption, design: .monospaced, weight: .bold))
                .foregroundStyle(CharioxPalette.muted)
                .frame(width: 82, alignment: .leading)
            Text(value)
                .font(.system(.caption, design: .monospaced))
                .foregroundStyle(.white)
                .lineLimit(2)
                .truncationMode(.middle)
        }
    }
}

struct StatusPill: View {
    enum Tone {
        case accent
        case danger
        case muted
    }

    let label: String
    let tone: Tone

    var body: some View {
        Text(label)
            .font(.system(.caption2, design: .monospaced, weight: .bold))
            .padding(.horizontal, 9)
            .padding(.vertical, 6)
            .foregroundStyle(color)
            .background(color.opacity(0.12))
            .clipShape(.rect(cornerRadius: 6))
    }

    private var color: Color {
        switch tone {
        case .accent:
            CharioxPalette.orange
        case .danger:
            CharioxPalette.red
        case .muted:
            CharioxPalette.muted
        }
    }
}

struct GlobalFooter: View {
    let state: ConnectionState
    let streamState: EventStreamState
    let session: RuntimeSession?
    let attachment: RuntimeAttachment?
    let message: String

    var body: some View {
        HStack(spacing: 12) {
            Text(state.label)
                .font(.system(.caption2, design: .monospaced, weight: .bold))
                .foregroundStyle(state == .failed ? CharioxPalette.red : CharioxPalette.orange)
            Text(streamState.label)
                .font(.system(.caption2, design: .monospaced, weight: .bold))
                .foregroundStyle(attachment == nil ? CharioxPalette.muted : CharioxPalette.orange)
            Text(session?.shortDisplayID ?? "no session")
                .font(.system(.caption2, design: .monospaced))
                .foregroundStyle(CharioxPalette.muted)
            Text(message)
                .font(.system(.caption2, design: .monospaced))
                .foregroundStyle(CharioxPalette.muted)
                .lineLimit(1)
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 8)
        .background(CharioxPalette.background)
        .overlay(alignment: .top) {
            Rectangle()
                .fill(CharioxPalette.border)
                .frame(height: 1)
        }
        .accessibilityIdentifier("global-footer")
    }
}

struct CharioxCommandButtonStyle: ButtonStyle {
    var primary = false

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.system(.caption, design: .monospaced, weight: .semibold))
            .padding(.horizontal, 12)
            .padding(.vertical, 9)
            .foregroundStyle(primary ? CharioxPalette.background : .white)
            .background(primary ? CharioxPalette.orange : CharioxPalette.field)
            .clipShape(.rect(cornerRadius: 8))
            .opacity(configuration.isPressed ? 0.7 : 1)
    }
}

struct AgentIconButtonStyle: ButtonStyle {
    var primary = false

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.system(.body, weight: .semibold))
            .frame(width: 34, height: 34)
            .foregroundStyle(primary ? CharioxPalette.background : .white)
            .background(primary ? CharioxPalette.orange : CharioxPalette.field)
            .clipShape(.rect(cornerRadius: 7))
            .opacity(configuration.isPressed ? 0.7 : 1)
    }
}
