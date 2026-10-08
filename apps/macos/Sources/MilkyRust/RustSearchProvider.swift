import CMilkyFFI
import Foundation
import MilkyNative

/// All handle access is actor-isolated. No pointer escapes and no unchecked
/// Sendable conformance is needed. Synchronous requests finish before shutdown.
public actor RustSearchProvider: SearchProvider, SelectionEventRecording {
    private var engine: EngineHandle?
    private var closed = false
    private let limit: Int

    public init(limit: Int = 20) { self.limit = max(0, limit) }

    public func prepare() throws {
        guard !closed else { throw BridgeError.closed }
        if engine == nil { engine = try EngineHandle() }
    }

    public func search(query: String) async throws -> [AppResult] {
        try Task.checkCancellation()
        try prepare()
        let request = try JSONEncoder().encode(SearchRequest(query: query, limit: limit))
        let copied = try engine!.request(request)
        try Task.checkCancellation()
        return try Self.decode(copied, expectedQuery: query)
    }

    public func recordSelection(_ event: MilkyNative.SelectionEvent) async throws {
        try Task.checkCancellation()
        try prepare()
        let request = try JSONEncoder().encode(RecordSelectionRequest(event: event))
        let copied = try engine!.request(request)
        try Self.validateRecorded(copied)
    }

    static func validateRecorded(_ data: Data) throws {
        do {
            let reply = try JSONDecoder().decode(Reply.self, from: data)
            switch reply.kind {
            case "recorded": return
            case "error":
                guard let reason = reply.reason, let message = reply.message else { throw BridgeError.invalidResponse }
                throw BridgeError.engine(reason: reason, message: message)
            default: throw BridgeError.invalidResponse
            }
        } catch let error as BridgeError { throw error }
        catch { throw BridgeError.invalidResponse }
    }

    public func shutdown() {
        closed = true
        engine = nil
    }

    static func validateVersion(_ actual: UInt32) throws {
        guard actual == 3 else { throw BridgeError.version(actual) }
    }

    static func decode(_ data: Data, expectedQuery: String) throws -> [AppResult] {
        do {
            let reply = try JSONDecoder().decode(Reply.self, from: data)
            switch reply.kind {
            case "error":
                guard let reason = reply.reason, let message = reply.message else { throw BridgeError.invalidResponse }
                throw BridgeError.engine(reason: reason, message: message)
            case "search":
                guard reply.query == expectedQuery, let results = reply.results else { throw BridgeError.invalidResponse }
                var identities = Set<String>()
                return try results.map { result in
                    guard !result.id.isEmpty, !result.id.contains("\0"), identities.insert(result.id).inserted else {
                        throw BridgeError.invalidResponse
                    }
                    let action: ResultAction
                    switch (result.kind, result.action) {
                    case (.app, .launch(let path)):
                        guard path.hasPrefix("/"), !path.contains("\0"), result.id == "app:\(path)" else {
                            throw BridgeError.invalidResponse
                        }
                        action = .launch(URL(fileURLWithPath: path))
                    case (.setting, .openURL(let text)):
                        guard let url = URL(string: text), let scheme = url.scheme, !scheme.isEmpty else {
                            throw BridgeError.invalidResponse
                        }
                        action = .openURL(url)
                    default: throw BridgeError.invalidResponse
                    }
                    return AppResult(id: result.id, kind: result.kind, title: result.title,
                                     subtitle: result.subtitle, action: action)
                }
            default: throw BridgeError.invalidResponse
            }
        } catch let error as BridgeError { throw error }
        catch { throw BridgeError.invalidResponse }
    }
}

private struct SearchRequest: Encodable {
    let op = "search"
    let query: String
    let limit: Int
}

private struct RecordSelectionRequest: Encodable {
    let op = "record_selection"
    let query: String
    let shown: [String]
    let selected: String
    let outcome: String

    init(event: MilkyNative.SelectionEvent) {
        query = event.query
        shown = event.shown
        selected = event.selected
        outcome = event.outcome.rawValue
    }
}

private struct Reply: Decodable {
    let kind: String
    let query: String?
    let results: [Item]?
    // `record_selection` success has kind `recorded` and no other fields.
    let reason: String?
    let message: String?

    struct Item: Decodable {
        let id: String
        let kind: ResultKind
        let title: String
        let subtitle: String
        let action: WireAction
        // Match kind is deliberately not interpreted by the UI; Rust owns order.
    }
}

private enum WireAction: Decodable {
    case launch(String)
    case openURL(String)

    private enum CodingKeys: String, CodingKey { case type, path, url }
    private enum ActionType: String, Decodable { case launch, openURL = "open_url" }

    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        switch try values.decode(ActionType.self, forKey: .type) {
        case .launch: self = .launch(try values.decode(String.self, forKey: .path))
        case .openURL: self = .openURL(try values.decode(String.self, forKey: .url))
        }
    }
}

private final class EngineHandle {
    let pointer: OpaquePointer

    init() throws {
        let actual = milky_abi_version()
        try RustSearchProvider.validateVersion(actual)
        guard let pointer = milky_engine_new() else { throw BridgeError.initialization }
        self.pointer = pointer
    }

    func request(_ data: Data) throws -> Data {
        let json = String(decoding: data, as: UTF8.self)
        return try json.withCString { request in
            guard let response = milky_engine_request(pointer, request) else { throw BridgeError.invalidResponse }
            defer { milky_string_free(response) }
            // Copy the UTF-8 bytes before Rust frees its allocation. Decode later.
            return Data(bytes: response, count: strlen(response))
        }
    }

    deinit { milky_engine_free(pointer) }
}

public enum BridgeError: LocalizedError {
    case version(UInt32)
    case initialization
    case invalidResponse
    case engine(reason: String, message: String)
    case closed

    public var errorDescription: String? {
        switch self {
        case .version: return "The search engine is out of date. Quit Milky and rebuild with scripts/run.sh."
        case .initialization: return "The search engine couldn’t start. Retry or restart Milky."
        case .invalidResponse: return "The search engine returned an unreadable response. Rebuild Milky and try again."
        case .engine(_, let message): return "Search failed. \(message)"
        case .closed: return "The search engine has shut down. Restart Milky."
        }
    }
}
