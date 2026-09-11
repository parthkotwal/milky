import CMilkyFFI
import Foundation

/// Safe Swift front end for Milky's Rust core.
///
/// This is the only type in the app that touches the C ABI. Everything above it
/// sees ordinary Swift values and thrown errors. Two invariants make that safe:
///
/// - the handle is created in `init` and destroyed in `deinit`, so it cannot
///   outlive this object or be freed twice;
/// - every string the Rust side allocates is released in a `defer` on the same
///   line of reasoning that received it.
///
/// Create one at app launch and keep it alive. Construction is where any real
/// startup cost will eventually live, and a launcher must be warm before the
/// user presses the hotkey.
public final class MilkyEngine {
    private let handle: OpaquePointer
    private let encoder = JSONEncoder()
    private let decoder = JSONDecoder()

    /// - Parameter configuration: engine config as JSON. `nil` uses defaults.
    public init(configuration: String? = nil) throws {
        let found = milky_abi_version()
        guard found == UInt32(MILKY_ABI_VERSION) else {
            throw MilkyError.abiMismatch(expected: UInt32(MILKY_ABI_VERSION), found: found)
        }

        var created: OpaquePointer?
        // Rust returns NULL on success, or an owned error string on failure.
        if let error = milky_engine_new(configuration, &created) {
            defer { milky_string_free(error) }
            throw MilkyError.engineUnavailable(String(cString: error))
        }
        guard let created else {
            throw MilkyError.engineUnavailable("engine_new reported success but produced no handle")
        }
        self.handle = created
    }

    deinit {
        milky_engine_free(handle)
    }

    /// Liveness and version probe. Call once at startup.
    public func health() throws -> Health {
        let response = try send(HealthRequest())
        guard case .health(let health) = response else {
            throw MilkyError.malformedResponse("expected a health response")
        }
        return health
    }

    /// Run a search.
    ///
    /// - Parameter limit: maximum results, or `nil` for the engine's default.
    public func search(_ query: String, limit: Int? = nil) throws -> SearchResults {
        let response = try send(SearchRequest(query: query, limit: limit))
        guard case .search(let results) = response else {
            throw MilkyError.malformedResponse("expected a search response")
        }
        return results
    }

    /// Send a raw JSON request. Escape hatch for debugging and for operations
    /// MilkyKit has not grown a typed method for yet.
    public func requestJSON(_ json: String) throws -> String {
        guard let raw = milky_engine_request(handle, json) else {
            // Documented as impossible; treated as a failure rather than trusted.
            throw MilkyError.malformedResponse("engine returned no response")
        }
        defer { milky_string_free(raw) }
        return String(cString: raw)
    }

    private func send(_ request: some Encodable) throws -> EngineResponse {
        let requestData = try encoder.encode(request)
        guard let requestJSON = String(data: requestData, encoding: .utf8) else {
            throw MilkyError.malformedResponse("request was not valid UTF-8")
        }

        let responseJSON = try requestJSON.withCString { pointer -> String in
            guard let raw = milky_engine_request(handle, pointer) else {
                throw MilkyError.malformedResponse("engine returned no response")
            }
            defer { milky_string_free(raw) }
            return String(cString: raw)
        }

        let response: EngineResponse
        do {
            response = try decoder.decode(EngineResponse.self, from: Data(responseJSON.utf8))
        } catch {
            throw MilkyError.malformedResponse("\(error) — payload: \(responseJSON)")
        }

        // Surface engine-reported failures as thrown errors so callers do not
        // have to check two things.
        if case .failure(let failure) = response {
            throw MilkyError.engine(failure)
        }
        return response
    }
}
