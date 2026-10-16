import AppKit
import CoreGraphics
import Foundation
import MilkyNative

/// Observes app-level foreground intervals without capturing input content.
/// It also owns the short-lived foreground timer for a successful launcher pick.
@MainActor
final class AppActivityObserver {
    private struct OpenInterval {
        let app: String
        let startedAt: Date
        var lastInputAt: Date
    }
    private struct Dwell {
        let session: InteractionSession
        let resultID: String
        var app: String?
        var startedAt: Date?
        var lastInputAt: Date?
    }

    private let sendFocus: ([InteractionFocusInterval]) -> Void
    private let sendDwell: (InteractionSession) -> Void
    private var observers: [NSObjectProtocol] = []
    private var timer: Timer?
    private var current: OpenInterval?
    private var pendingFocus: [InteractionFocusInterval] = []
    private var dwell: Dwell?
    private var lastFlush = Date()
    private var pausedForSleep = false
    private let idleLimit: TimeInterval = 120
    private let dwellLimit: TimeInterval = 30 * 60
    private let selfBundleID = Bundle.main.bundleIdentifier

    init(sendFocus: @escaping ([InteractionFocusInterval]) -> Void,
         sendDwell: @escaping (InteractionSession) -> Void) {
        self.sendFocus = sendFocus
        self.sendDwell = sendDwell
    }

    func start() {
        guard timer == nil else { return }
        let workspace = NSWorkspace.shared.notificationCenter
        observers.append(workspace.addObserver(forName: NSWorkspace.didActivateApplicationNotification,
                                               object: nil, queue: .main) { [weak self] note in
            let app = note.userInfo?[NSWorkspace.applicationUserInfoKey] as? NSRunningApplication
            MainActor.assumeIsolated { self?.activated(app) }
        })
        observers.append(workspace.addObserver(forName: NSWorkspace.didTerminateApplicationNotification,
                                               object: nil, queue: .main) { [weak self] note in
            let app = note.userInfo?[NSWorkspace.applicationUserInfoKey] as? NSRunningApplication
            MainActor.assumeIsolated { self?.terminated(app) }
        })
        observers.append(workspace.addObserver(forName: NSWorkspace.didHideApplicationNotification,
                                               object: nil, queue: .main) { [weak self] note in
            let app = note.userInfo?[NSWorkspace.applicationUserInfoKey] as? NSRunningApplication
            MainActor.assumeIsolated { self?.hidden(app) }
        })
        observers.append(workspace.addObserver(forName: NSWorkspace.screensDidSleepNotification,
                                               object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.sleep() }
        })
        observers.append(workspace.addObserver(forName: NSWorkspace.screensDidWakeNotification,
                                               object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.wake() }
        })
        observers.append(workspace.addObserver(forName: NSWorkspace.sessionDidResignActiveNotification,
                                               object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.lock() }
        })
        observers.append(workspace.addObserver(forName: NSWorkspace.sessionDidBecomeActiveNotification,
                                               object: nil, queue: .main) { [weak self] _ in
            MainActor.assumeIsolated { self?.wake() }
        })
        if let app = NSWorkspace.shared.frontmostApplication { activated(app) }
        timer = Timer.scheduledTimer(withTimeInterval: 5, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.tick() }
        }
    }

    func beginDwell(session: InteractionSession, resultID: String) {
        guard !session.sessionID.isEmpty else { return }
        let actionAt = Date()
        dwell = Dwell(session: session, resultID: resultID, app: nil,
                      startedAt: actionAt, lastInputAt: actionAt)
        let front = NSWorkspace.shared.frontmostApplication
        if let front, isExternalRegular(front) { beginDwellFront(front, at: Date()) }
    }

    func stop() {
        timer?.invalidate(); timer = nil
        closeCurrent(at: Date(), endedBy: "shutdown")
        if dwell != nil { finishDwell(at: Date(), ended: "hidden") }
        flushFocus()
        for observer in observers { NSWorkspace.shared.notificationCenter.removeObserver(observer) }
        observers.removeAll()
    }

    private func activated(_ app: NSRunningApplication?) {
        guard !pausedForSleep else { return }
        let now = Date()
        if let current, current.app != appID(app) { closeCurrent(at: now, endedBy: "switch") }
        guard let app, isExternalRegular(app), let id = appID(app) else {
            if let dwell, dwell.app != nil { finishDwell(at: now, ended: "switched") }
            return
        }
        if current == nil { current = OpenInterval(app: id, startedAt: now, lastInputAt: now) }
        if dwell != nil {
            if dwell?.app == nil { beginDwellFront(app, at: now) }
            else if dwell?.app != id { finishDwell(at: now, ended: "switched") }
        }
    }

    private func terminated(_ app: NSRunningApplication?) {
        guard let id = appID(app) else { return }
        let now = Date()
        if current?.app == id { closeCurrent(at: now, endedBy: "quit") }
        if dwell?.app == id { finishDwell(at: now, ended: "quit") }
    }

    private func hidden(_ app: NSRunningApplication?) {
        guard let id = appID(app), dwell?.app == id else { return }
        finishDwell(at: Date(), ended: "hidden")
    }

    private func beginDwellFront(_ app: NSRunningApplication, at date: Date) {
        guard let id = appID(app), isExternalRegular(app), var value = dwell else { return }
        value.app = id; value.startedAt = date; value.lastInputAt = date; dwell = value
    }

    private func tick() {
        let now = Date()
        let measuredIdle = min(keyboardIdleSeconds(), pointerIdleSeconds())
        let idle = measuredIdle.isFinite ? min(max(0, measuredIdle), 24 * 60 * 60) : 0
        let lastInput = now.addingTimeInterval(-idle)

        if var interval = current {
            if idle >= idleLimit {
                interval.lastInputAt = lastInput
                current = interval
                closeCurrent(at: lastInput, endedBy: "idle")
            } else {
                interval.lastInputAt = now
                current = interval
            }
        } else if idle < 5, let front = NSWorkspace.shared.frontmostApplication, isExternalRegular(front), let id = appID(front) {
            current = OpenInterval(app: id, startedAt: lastInput, lastInputAt: now)
        }

        if var active = dwell, let startedAt = active.startedAt {
            if now.timeIntervalSince(startedAt) >= dwellLimit {
                finishDwell(at: startedAt.addingTimeInterval(dwellLimit), ended: "limit")
            } else if idle >= idleLimit {
                active.lastInputAt = lastInput; dwell = active
                finishDwell(at: lastInput, ended: "idle")
            } else {
                active.lastInputAt = now; dwell = active
            }
        }
        if now.timeIntervalSince(lastFlush) >= 300 { flushFocus() }
    }

    private func keyboardIdleSeconds() -> TimeInterval {
        let key = CGEventSource.secondsSinceLastEventType(.combinedSessionState, eventType: .keyDown)
        return key.isFinite && key >= 0 ? key : .infinity
    }
    private func pointerIdleSeconds() -> TimeInterval {
        let mouse = CGEventSource.secondsSinceLastEventType(.combinedSessionState, eventType: .mouseMoved)
        let left = CGEventSource.secondsSinceLastEventType(.combinedSessionState, eventType: .leftMouseDown)
        let right = CGEventSource.secondsSinceLastEventType(.combinedSessionState, eventType: .rightMouseDown)
        let scroll = CGEventSource.secondsSinceLastEventType(.combinedSessionState, eventType: .scrollWheel)
        return [mouse, left, right, scroll].filter { $0.isFinite && $0 >= 0 }.min() ?? .infinity
    }

    private func closeCurrent(at date: Date, endedBy: String) {
        guard let interval = current else { return }
        current = nil
        let end = max(interval.startedAt, date)
        pendingFocus.append(InteractionFocusInterval(app: interval.app,
            startedAtMs: epochMS(interval.startedAt), endedAtMs: epochMS(end), endedBy: endedBy))
        if pendingFocus.count >= 1000 { flushFocus() }
    }

    private func finishDwell(at date: Date, ended: String) {
        guard let value = dwell, let startedAt = value.startedAt else { dwell = nil; return }
        dwell = nil
        let end = max(startedAt, date)
        let eventT = max(0, Int(end.timeIntervalSince1970 * 1000) - Int(value.session.startedAtMs))
        let frontMS = value.app == nil ? 0 : max(0, Int(end.timeIntervalSince(startedAt) * 1000))
        let report = value.session.addingDwell(t: eventT, resultID: value.resultID, app: value.app,
                                               frontMS: frontMS, ended: ended)
        sendDwell(report)
    }

    private func sleep() {
        pausedForSleep = true
        closeCurrent(at: Date(), endedBy: "sleep")
        if dwell != nil { finishDwell(at: Date(), ended: "sleep") }
        flushFocus()
    }
    private func lock() {
        closeCurrent(at: Date(), endedBy: "lock")
        if dwell != nil { finishDwell(at: Date(), ended: "sleep") }
        flushFocus()
    }
    private func wake() {
        pausedForSleep = false
        if let app = NSWorkspace.shared.frontmostApplication { activated(app) }
    }

    private func flushFocus() {
        guard !pendingFocus.isEmpty else { lastFlush = Date(); return }
        let batch = Array(pendingFocus.prefix(1000))
        pendingFocus.removeFirst(batch.count)
        lastFlush = Date()
        sendFocus(batch)
        if !pendingFocus.isEmpty { flushFocus() }
    }
    private func isExternalRegular(_ app: NSRunningApplication) -> Bool {
        app.activationPolicy == .regular && app.processIdentifier != ProcessInfo.processInfo.processIdentifier &&
            (selfBundleID == nil || app.bundleIdentifier != selfBundleID)
    }
    private func appID(_ app: NSRunningApplication?) -> String? {
        guard let app, isExternalRegular(app), let path = app.bundleURL?.path, path.hasPrefix("/") else { return nil }
        return "app:\(path)"
    }
    private func epochMS(_ date: Date) -> Int64 { Int64(date.timeIntervalSince1970 * 1000) }
}
