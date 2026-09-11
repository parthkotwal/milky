import Foundation

// Swift mirrors of the types in crates/milky-core/src/api.rs. These are the
// second place the two languages must agree; unlike the C header, a mismatch
// here surfaces as a clean decoding error rather than as corruption.

/// Version of the request/response contract this client speaks.
public let milkyAPIVersion: UInt32 = 1

public enum CandidateKind: String, Codable, Sendable {
    case application
    case file
    case action
    case webSearch = "web_search"
    case placeholder
}

public struct Candidate: Codable, Sendable, Identifiable {
    public let id: String
    public let title: String
    public let subtitle: String?
    public let kind: CandidateKind
    public let score: Float
}

public struct Health: Codable, Sendable {
    public let engineVersion: String
    public let apiVersion: UInt32
    public let ready: Bool
    public let note: String?

    private enum CodingKeys: String, CodingKey {
        case engineVersion = "engine_version"
        case apiVersion = "api_version"
        case ready
        case note
    }
}

public struct SearchResults: Codable, Sendable {
    /// The query the engine answered. Compare against what is in the search
    /// field to drop responses for keystrokes the user has already moved past.
    public let query: String
    public let tookMicros: UInt64
    public let candidates: [Candidate]

    private enum CodingKeys: String, CodingKey {
        case query
        case tookMicros = "took_micros"
        case candidates
    }
}

/// An error the engine reported. Distinct from `MilkyError`, which covers
/// failures of the boundary itself.
public struct EngineErrorResponse: Codable, Sendable {
    public enum Reason: String, Codable, Sendable {
        case badRequest = "bad_request"
        case internalError = "internal"
        case panic
    }

    public let reason: Reason
    public let message: String
}

/// Anything that can go wrong on the Swift side of the boundary.
public enum MilkyError: Error, CustomStringConvertible {
    /// The engine could not be constructed.
    case engineUnavailable(String)
    /// The linked Rust library speaks a different ABI than this build expects.
    case abiMismatch(expected: UInt32, found: UInt32)
    /// The engine returned a well-formed error response.
    case engine(EngineErrorResponse)
    /// The response did not match the contract, or was the wrong kind.
    case malformedResponse(String)

    public var description: String {
        switch self {
        case .engineUnavailable(let message):
            return "Milky engine unavailable: \(message)"
        case .abiMismatch(let expected, let found):
            return """
                Milky ABI mismatch: MilkyKit expects \(expected) but libmilky_ffi \
                reports \(found). Rebuild the Rust core.
                """
        case .engine(let response):
            return "Milky engine error (\(response.reason.rawValue)): \(response.message)"
        case .malformedResponse(let message):
            return "Malformed response from Milky engine: \(message)"
        }
    }
}

/// A decoded response, discriminated the same way the Rust enum is.
enum EngineResponse: Decodable {
    case health(Health)
    case search(SearchResults)
    case failure(EngineErrorResponse)

    private enum CodingKeys: String, CodingKey {
        case kind
    }

    init(from decoder: any Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        let kind = try container.decode(String.self, forKey: .kind)
        // Rust tags these enums internally, so the variant's fields sit at the
        // top level next to "kind" and decode straight from the same decoder.
        switch kind {
        case "health":
            self = .health(try Health(from: decoder))
        case "search":
            self = .search(try SearchResults(from: decoder))
        case "error":
            self = .failure(try EngineErrorResponse(from: decoder))
        default:
            throw DecodingError.dataCorruptedError(
                forKey: .kind,
                in: container,
                debugDescription: "unknown response kind '\(kind)'"
            )
        }
    }
}

// Request bodies. Encoded rather than string-built so escaping is never our
// problem.
struct HealthRequest: Encodable {
    let op = "health"
}

struct SearchRequest: Encodable {
    let op = "search"
    let query: String
    let limit: Int?
}
