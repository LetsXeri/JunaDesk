import AppKit
import AVFoundation
import CoreGraphics
import QuartzCore
import IOKit.ps
import Foundation

struct Wallpaper: Codable {
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
    let hasBattery: Bool?
    let rendered: [String: Wallpaper]
    let errors: [String: String]
}

final class WallpaperWindow: NSWindow {
    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }
}

private final class FrameClock: NSObject {
    weak var surface: Surface?

    @objc func refresh(_ link: CADisplayLink) {
        surface?.render(at: link.targetTimestamp)
    }
}

final class Surface {
    let window: NSWindow
    let player = AVQueuePlayer()
    private let layer = AVSampleBufferDisplayLayer()
    let looper: AVPlayerLooper
    let path: String
    var paused = true
    private(set) var shown = false
    private var prerolled = false
    private weak var renderedItem: AVPlayerItem?
    private var outputs: [ObjectIdentifier: AVPlayerItemVideoOutput] = [:]
    private var format: CMVideoFormatDescription?
    private let frameClock = FrameClock()
    private var displayLink: CADisplayLink?
    private var flushing = false

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
        layer.videoGravity = .resizeAspectFill
        layer.backgroundColor = NSColor.black.cgColor
        let view = NSView(frame: CGRect(origin: .zero, size: screen.frame.size))
        view.wantsLayer = true
        view.layer = layer
        window.contentView = view

        let item = AVPlayerItem(url: URL(fileURLWithPath: path))
        looper = AVPlayerLooper(player: player, templateItem: item)
        attachOutputs()
        frameClock.surface = self
        let link = screen.displayLink(target: frameClock, selector: #selector(FrameClock.refresh(_:)))
        displayLink = link
        link.add(to: .main, forMode: .common)
    }

    func update(frame: CGRect, paused: Bool) {
        if paused && !shown && !prerolled && player.status == .readyToPlay {
            prerolled = true
            player.preroll(atRate: 1) { _ in }
        }
        if frame != window.frame { window.setFrame(frame, display: true) }
        guard self.paused != paused else { return }
        self.paused = paused
        if paused {
            player.pause()
        } else {
            player.cancelPendingPrerolls()
            player.play()
        }
        displayLink?.isPaused = paused && shown
    }

    private func attachOutputs() {
        // The looper copies the template item without copying its video output.
        var items = looper.loopingPlayerItems
        if let current = player.currentItem { items.append(current) }
        let active = Set(items.map(ObjectIdentifier.init))
        outputs = outputs.filter { active.contains($0.key) }

        for item in items {
            let id = ObjectIdentifier(item)
            guard outputs[id] == nil else { continue }
            let output = AVPlayerItemVideoOutput(pixelBufferAttributes: [
                kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange
            ])
            item.add(output)
            outputs[id] = output
        }
    }

    fileprivate func render(at hostTime: CFTimeInterval) {
        let renderer = layer.sampleBufferRenderer
        if !shown {
            let ready: Bool
            if #available(macOS 14.4, *) {
                ready = layer.isReadyForDisplay
            } else {
                ready = renderer.status == .rendering
            }
            if ready {
                window.orderFrontRegardless()
                shown = true
                if paused {
                    displayLink?.isPaused = true
                    return
                }
            }
        }
        guard let item = player.currentItem else { return }
        if item !== renderedItem {
            // Refresh outputs for the upcoming replicas at each item transition.
            attachOutputs()
            renderedItem = item
        }
        guard let output = outputs[ObjectIdentifier(item)] else { return }
        if renderer.requiresFlushToResumeDecoding {
            guard !flushing else { return }
            flushing = true
            renderer.flush(removingDisplayedImage: false) { [weak self] in
                DispatchQueue.main.async { self?.flushing = false }
            }
            return
        }
        let itemTime = output.itemTime(forHostTime: hostTime)
        guard !flushing, renderer.isReadyForMoreMediaData, itemTime.isNumeric,
              output.hasNewPixelBuffer(forItemTime: itemTime),
              let pixel = output.copyPixelBuffer(forItemTime: itemTime, itemTimeForDisplay: nil)
        else { return }

        if format.map({ !CMVideoFormatDescriptionMatchesImageBuffer($0, imageBuffer: pixel) }) ?? true {
            format = nil
            guard CMVideoFormatDescriptionCreateForImageBuffer(
                allocator: kCFAllocatorDefault, imageBuffer: pixel, formatDescriptionOut: &format
            ) == noErr else { return }
        }
        guard let format else { return }
        var timing = CMSampleTimingInfo(duration: .invalid, presentationTimeStamp: itemTime, decodeTimeStamp: .invalid)
        var sample: CMSampleBuffer?
        guard CMSampleBufferCreateReadyWithImageBuffer(
            allocator: kCFAllocatorDefault, imageBuffer: pixel,
            formatDescription: format, sampleTiming: &timing, sampleBufferOut: &sample
        ) == noErr, let sample,
              let attachments = CMSampleBufferGetSampleAttachmentsArray(sample, createIfNecessary: true)
        else { return }
        let attachment = unsafeBitCast(CFArrayGetValueAtIndex(attachments, 0), to: CFMutableDictionary.self)
        CFDictionarySetValue(
            attachment, Unmanaged.passUnretained(kCMSampleAttachmentKey_DisplayImmediately).toOpaque(),
            Unmanaged.passUnretained(kCFBooleanTrue).toOpaque()
        )
        // Keep this layer and its last frame across item changes, even if the next
        // decoder frame arrives a refresh later than the end of the previous item.
        renderer.enqueue(sample)
    }

    var error: String? {
        (looper.error ?? player.currentItem?.error ?? player.error)?.localizedDescription
    }

    deinit {
        displayLink?.invalidate()
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
        var rendered: [String: Wallpaper] = [:]
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
                if surface.shown && surface.window.isVisible {
                    rendered[id] = Wallpaper(path: surface.path, paused: surface.paused)
                }
            }
        }
        let connected = Set(monitors.map(\.id))
        for id in Array(surfaces.keys) where !connected.contains(id) {
            surfaces.removeValue(forKey: id)
        }

        let snapshot = IOPSCopyPowerSourcesInfo()?.takeRetainedValue()
        let battery = snapshot.flatMap { IOPSGetProvidingPowerSourceType($0)?.takeUnretainedValue() as String? }
        let hasBattery = snapshot.flatMap { info in
            (IOPSCopyPowerSourcesList(info)?.takeRetainedValue() as? [CFTypeRef])?.contains { source in
                let description = IOPSGetPowerSourceDescription(info, source)?.takeUnretainedValue() as? [String: Any]
                return description?[kIOPSTypeKey] as? String == kIOPSInternalBatteryType
            }
        }
        if let data = try? JSONEncoder().encode(Observation(
            monitors: monitors, onBattery: battery == kIOPSBatteryPowerValue, hasBattery: hasBattery,
            rendered: rendered, errors: errors
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
