import AppKit
import AVFoundation
import CoreGraphics
import IOKit.ps
import Foundation

struct Wallpaper: Decodable {
    let path: String
    let paused: Bool
}

struct Request: Decodable {
    let wallpapers: [String: Wallpaper]
}

struct Display: Encodable {
    let id: String
    let name: String
    let width: Int
    let height: Int
    let x: Int
    let y: Int
    let primary: Bool
    let fullscreen: Bool
}

struct Observation: Encodable {
    let monitors: [Display]
    let onBattery: Bool
    let errors: [String: String]
}

final class WallpaperWindow: NSWindow {
    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }
}

final class Surface {
    let window: NSWindow
    let player = AVQueuePlayer()
    let layer: AVPlayerLayer
    let looper: AVPlayerLooper
    let path: String
    var paused = true
    private var shown = false
    private var prerolled = false

    init(screen: NSScreen, path: String) {
        self.path = path
        window = WallpaperWindow(
            contentRect: screen.frame, styleMask: .borderless, backing: .buffered, defer: false
        )
        window.isReleasedWhenClosed = false
        window.setFrame(screen.frame, display: false)
        window.level = NSWindow.Level(rawValue: Int(CGWindowLevelForKey(.desktopIconWindow)) - 1)
        window.collectionBehavior = [.canJoinAllSpaces, .stationary, .ignoresCycle]
        window.ignoresMouseEvents = true
        window.hasShadow = false
        window.backgroundColor = .black
        window.hidesOnDeactivate = false
        window.sharingType = .none

        player.isMuted = true
        layer = AVPlayerLayer(player: player)
        layer.videoGravity = .resizeAspectFill
        layer.backgroundColor = NSColor.black.cgColor
        let view = NSView(frame: CGRect(origin: .zero, size: screen.frame.size))
        view.wantsLayer = true
        view.layer = layer
        window.contentView = view

        let item = AVPlayerItem(url: URL(fileURLWithPath: path))
        looper = AVPlayerLooper(player: player, templateItem: item)
    }

    func update(frame: CGRect, paused: Bool) {
        if paused && !shown && !prerolled && player.status == .readyToPlay {
            prerolled = true
            player.preroll(atRate: 1) { _ in }
        }
        if frame != window.frame { window.setFrame(frame, display: true) }
        if !shown && layer.isReadyForDisplay {
            window.orderFrontRegardless()
            shown = true
        }
        guard self.paused != paused else { return }
        self.paused = paused
        if paused {
            player.pause()
        } else {
            player.cancelPendingPrerolls()
            player.play()
        }
    }

    var error: String? {
        (looper.error ?? player.currentItem?.error ?? player.error)?.localizedDescription
    }

    deinit {
        player.cancelPendingPrerolls()
        player.pause()
        looper.disableLooping()
        player.removeAllItems()
        window.close()
    }
}

final class Desktop {
    private var surfaces: [String: Surface] = [:]
    var desired: [String: Wallpaper] = [:]

    func tick() {
        let windows = CGWindowListCopyWindowInfo(
            [.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID
        ) as? [[String: Any]] ?? []
        var monitors: [Display] = []
        var errors: [String: String] = [:]

        for screen in NSScreen.screens {
            guard let number = screen.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? NSNumber else {
                continue
            }
            let displayID = number.uint32Value
            guard let uuid = CGDisplayCreateUUIDFromDisplayID(displayID)?.takeRetainedValue() else { continue }
            let id = CFUUIDCreateString(nil, uuid) as String
            let bounds = CGDisplayBounds(displayID)
            let mode = CGDisplayCopyDisplayMode(displayID)
            monitors.append(Display(
                id: id, name: screen.localizedName,
                width: mode?.pixelWidth ?? CGDisplayPixelsWide(displayID),
                height: mode?.pixelHeight ?? CGDisplayPixelsHigh(displayID),
                x: Int(bounds.minX), y: Int(bounds.minY), primary: CGDisplayIsMain(displayID) != 0,
                fullscreen: isFullscreen(bounds: bounds, windows: windows)
            ))
            guard let wallpaper = desired[id] else {
                surfaces.removeValue(forKey: id)
                continue
            }
            if surfaces[id]?.path != wallpaper.path {
                surfaces[id] = Surface(screen: screen, path: wallpaper.path)
            }
            guard let surface = surfaces[id] else { continue }
            if let error = surface.error {
                errors[id] = error
                surface.window.orderOut(nil)
            } else {
                surface.update(frame: screen.frame, paused: wallpaper.paused)
            }
        }
        let connected = Set(monitors.map(\.id))
        for id in Array(surfaces.keys) where !connected.contains(id) {
            surfaces.removeValue(forKey: id)
        }

        let snapshot = IOPSCopyPowerSourcesInfo()?.takeRetainedValue()
        let battery = snapshot.flatMap { IOPSGetProvidingPowerSourceType($0)?.takeUnretainedValue() as String? }
        if let data = try? JSONEncoder().encode(Observation(
            monitors: monitors, onBattery: battery == kIOPSBatteryPowerValue, errors: errors
        )) {
            FileHandle.standardOutput.write(data + Data([10]))
        }
    }

    private func isFullscreen(bounds: CGRect, windows: [[String: Any]]) -> Bool {
        for window in windows {
            guard let owner = window[kCGWindowOwnerPID as String] as? pid_t, owner != getpid(),
                  let level = window[kCGWindowLayer as String] as? Int, level == 0,
                  let size = window[kCGWindowBounds as String] as? [String: Any],
                  let rect = CGRect(dictionaryRepresentation: size as CFDictionary),
                  rect.width > 100, rect.height > 100 else { continue }
            let intersection = rect.intersection(bounds)
            guard !intersection.isNull, intersection.width * intersection.height > bounds.width * bounds.height * 0.1
            else { continue }
            let tolerance: CGFloat = 2
            return rect.minX <= bounds.minX + tolerance && rect.minY <= bounds.minY + tolerance
                && rect.maxX >= bounds.maxX - tolerance && rect.maxY >= bounds.maxY - tolerance
        }
        return false
    }
}

let app = NSApplication.shared
app.setActivationPolicy(.accessory)
let desktop = Desktop()
DispatchQueue.global(qos: .utility).async {
    while let line = readLine() {
        if let request = try? JSONDecoder().decode(Request.self, from: Data(line.utf8)) {
            DispatchQueue.main.async {
                desktop.desired = request.wallpapers
                desktop.tick()
            }
        }
    }
    DispatchQueue.main.async { app.terminate(nil) }
}
let timer = Timer(timeInterval: 0.5, repeats: true) { _ in desktop.tick() }
RunLoop.main.add(timer, forMode: .common)
desktop.tick()
app.run()
