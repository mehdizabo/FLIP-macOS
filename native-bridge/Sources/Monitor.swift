import Foundation
import AppKit
import AVFoundation
import CoreGraphics
import QuartzCore
import AudioToolbox
import IOKit.hid
import Carbon.HIToolbox

private extension ParsedShortcut {
    var carbonModifiers: UInt32 {
        var mods: UInt32 = 0
        if command { mods |= UInt32(cmdKey) }
        if option { mods |= UInt32(optionKey) }
        if control { mods |= UInt32(controlKey) }
        if shift { mods |= UInt32(shiftKey) }
        return mods
    }
}

enum HapticIntensity: String {
    case light = "light"
    case medium = "medium"
    case firm = "firm"
    case strong = "strong"

    static func parse(_ raw: String?) -> HapticIntensity {
        guard let raw = raw?.trimmingCharacters(in: .whitespacesAndNewlines).lowercased(), !raw.isEmpty else {
            return .firm
        }
        switch raw {
        case "light", "1", "subtle":
            return .light
        case "medium", "2":
            return .medium
        case "firm", "3", "default":
            return .firm
        case "strong", "4", "5", "max":
            return .strong
        default:
            return .firm
        }
    }

    var logitechContinuousLevelByte: UInt8 {
        switch self {
        case .light: return 25
        case .medium: return 45
        case .firm: return 60   // Matches MX Master 4 hardware default (0x3C) & Logi Medium (Default)
        case .strong: return 85 // Crisp punch without LRA 100% rattle
        }
    }

    var logitechDiscreteLevelByte: UInt8 {
        switch self {
        case .light: return 25
        case .medium: return 50
        case .firm: return 75
        case .strong: return 100
        }
    }

    var audioTickGain: Float {
        switch self {
        case .light: return 0.58
        case .medium: return 0.78
        case .firm: return 0.94
        case .strong: return 1.00
        }
    }

    var audioFxGain: Float {
        switch self {
        case .light: return 0.78
        case .medium: return 0.88
        case .firm: return 0.96
        case .strong: return 1.00
        }
    }
}

enum ActivePointerSource {
    case trackpad
    case logitechMouse
    case otherMouse
    case unknown
}

final class ActivePointerDetector {
    static let shared = ActivePointerDetector()

    private typealias CreateClientFn = @convention(c) (CFAllocator?) -> CFTypeRef?
    private typealias SetMatchingMultipleFn = @convention(c) (CFTypeRef, CFArray) -> Void
    private typealias CopyServicesFn = @convention(c) (CFTypeRef) -> CFArray?
    private typealias CopyPropertyFn = @convention(c) (CFTypeRef, CFString) -> CFTypeRef?

    private static let matchList: [[String: Any]] = [
        ["DeviceUsagePage": 1, "DeviceUsage": 2],   // Mouse / pointer
        ["DeviceUsagePage": 13, "DeviceUsage": 12], // Apple Force Touch trackpad
        ["DeviceUsagePage": 13, "DeviceUsage": 5],  // Touchpad
    ]

    private let iokitHandle: UnsafeMutableRawPointer?
    private var client: CFTypeRef?
    private let createClient: CreateClientFn?
    private let setMatchingMultiple: SetMatchingMultipleFn?
    private let copyServices: CopyServicesFn?
    private let copyProperty: CopyPropertyFn?

    private init() {
        let handle = dlopen("/System/Library/Frameworks/IOKit.framework/IOKit", RTLD_LAZY)
        self.iokitHandle = handle
        guard let handle = handle,
              let createSym = dlsym(handle, "IOHIDEventSystemClientCreate"),
              let setMatchSym = dlsym(handle, "IOHIDEventSystemClientSetMatchingMultiple"),
              let copyServicesSym = dlsym(handle, "IOHIDEventSystemClientCopyServices"),
              let copyPropertySym = dlsym(handle, "IOHIDServiceClientCopyProperty") else {
            self.createClient = nil
            self.setMatchingMultiple = nil
            self.copyServices = nil
            self.copyProperty = nil
            self.client = nil
            return
        }

        self.createClient = unsafeBitCast(createSym, to: CreateClientFn.self)
        self.setMatchingMultiple = unsafeBitCast(setMatchSym, to: SetMatchingMultipleFn.self)
        self.copyServices = unsafeBitCast(copyServicesSym, to: CopyServicesFn.self)
        self.copyProperty = unsafeBitCast(copyPropertySym, to: CopyPropertyFn.self)

        refreshClient()
    }

    func refreshClient() {
        guard let createClient = createClient,
              let setMatchingMultiple = setMatchingMultiple,
              let newClient = createClient(kCFAllocatorDefault) else {
            self.client = nil
            return
        }
        setMatchingMultiple(newClient, Self.matchList as CFArray)
        self.client = newClient
    }

    func currentSource(logitechDirectReportTimestamp: UInt64 = 0) -> ActivePointerSource {
        guard let client = client,
              let copyServices = copyServices,
              let copyProperty = copyProperty,
              let services = copyServices(client) else {
            return logitechDirectReportTimestamp > 0 ? .logitechMouse : .unknown
        }

        var bestTimestamp: UInt64 = 0
        var bestKind: ActivePointerSource = .unknown
        var hasTrackpad = false
        var hasLogitech = false

        for service in services as NSArray {
            let svc = service as CFTypeRef
            let product = copyProperty(svc, "Product" as CFString) as? String
            let ioClass = copyProperty(svc, "IOClass" as CFString) as? String
            let vendorID = (copyProperty(svc, "VendorID" as CFString) as? NSNumber)?.intValue ?? 0
            let primaryUsagePage = (copyProperty(svc, "PrimaryUsagePage" as CFString) as? NSNumber)?.intValue ?? 0
            let primaryUsage = (copyProperty(svc, "PrimaryUsage" as CFString) as? NSNumber)?.intValue ?? 0
            let actuationSupported = (copyProperty(svc, "ActuationSupported" as CFString) as? NSNumber)?.boolValue ?? false
            let forceSupported = (copyProperty(svc, "ForceSupported" as CFString) as? NSNumber)?.boolValue ?? false
            let ts = (copyProperty(svc, "LastActivityTimestamp" as CFString) as? NSNumber)?.uint64Value ?? 0

            let lowerProd = (product ?? "").lowercased()
            let isTrackpad = !lowerProd.contains("mouse") && (
                actuationSupported ||
                forceSupported ||
                ioClass == "AppleMultitouchDevice" ||
                (primaryUsagePage == 13 && (primaryUsage == 12 || primaryUsage == 5)) ||
                lowerProd.contains("trackpad")
            )
            let kind: ActivePointerSource = isTrackpad
                ? .trackpad
                : (vendorID == 0x046d ? .logitechMouse : .otherMouse)
            if kind == .trackpad { hasTrackpad = true }
            if kind == .logitechMouse { hasLogitech = true }
            let effectiveTS = (kind == .logitechMouse) ? max(ts, logitechDirectReportTimestamp) : ts
            if effectiveTS > bestTimestamp {
                bestTimestamp = effectiveTS
                bestKind = kind
            }
        }

        if bestTimestamp == 0 {
            return logitechDirectReportTimestamp > 0 ? .logitechMouse : (hasTrackpad && !hasLogitech ? .trackpad : .unknown)
        }
        return bestKind
    }
}

private final class LogitechHIDContext {
    weak var driver: LogitechHapticDriver?
    let device: IOHIDDevice
    let productID: Int
    let reportBufferCapacity = 64
    let reportBuffer: UnsafeMutablePointer<UInt8>
    var featureIndices: [UInt8: UInt8] = [:]
    var lastConfiguredLevel: [UInt8: UInt8] = [:]
    var isContinuousLevel: Bool
    var supportsWhisperCollision: Bool
    var isOpen: Bool = false
    var powerSaveFlagsByDev: [UInt8: UInt8] = [:]
    var deferredPlayWorkItem: [UInt8: DispatchWorkItem] = [:]

    init(driver: LogitechHapticDriver, device: IOHIDDevice, productID: Int) {
        self.driver = driver
        self.device = device
        self.productID = productID
        self.isContinuousLevel = (productID == 0xb042)
        self.supportsWhisperCollision = (productID == 0xb042)
        self.reportBuffer = UnsafeMutablePointer<UInt8>.allocate(capacity: reportBufferCapacity)
        self.reportBuffer.initialize(repeating: 0, count: reportBufferCapacity)
        // Pre-seed featureIndex = 0x0B for MX Master 4 BLE (PID 0xb042) so immediate one-shot CLI calls work
        if productID == 0xb042 {
            featureIndices[0xFF] = 0x0B
            featureIndices[0x00] = 0x0B
        }
    }

    deinit {
        for item in deferredPlayWorkItem.values {
            item.cancel()
        }
        deferredPlayWorkItem.removeAll()
        reportBuffer.deinitialize(count: reportBufferCapacity)
        reportBuffer.deallocate()
    }
}

final class LogitechHapticDriver {
    static let shared = LogitechHapticDriver()

    private var manager: IOHIDManager?
    private var contexts: [ObjectIdentifier: LogitechHIDContext] = [:]
    private(set) var lastMouseReportTimestamp: UInt64 = 0
    private var lastTickPlayUptime: TimeInterval = 0
    private let tickCooldownSeconds: TimeInterval = 0.045
    private var lastReopenAttemptUptime: TimeInterval = 0
    private let reopenCooldownSeconds: TimeInterval = 2.0

    private init() {
        setupManager()
    }

    func reopenManager() {
        lastReopenAttemptUptime = ProcessInfo.processInfo.systemUptime
        for ctx in contexts.values {
            closeContext(ctx)
        }
        contexts.removeAll()
        if let oldMgr = manager {
            IOHIDManagerRegisterDeviceMatchingCallback(oldMgr, nil, nil)
            IOHIDManagerRegisterDeviceRemovalCallback(oldMgr, nil, nil)
            IOHIDManagerUnscheduleFromRunLoop(oldMgr, CFRunLoopGetMain(), CFRunLoopMode.commonModes.rawValue)
            _ = IOHIDManagerClose(oldMgr, IOOptionBits(kIOHIDOptionsTypeNone))
            manager = nil
        }
        setupManager()
    }

    private func closeContext(_ ctx: LogitechHIDContext) {
        for item in ctx.deferredPlayWorkItem.values {
            item.cancel()
        }
        ctx.deferredPlayWorkItem.removeAll()
        if ctx.isOpen {
            IOHIDDeviceRegisterInputReportCallback(
                ctx.device,
                ctx.reportBuffer,
                ctx.reportBufferCapacity,
                nil,
                nil
            )
            IOHIDDeviceUnscheduleFromRunLoop(ctx.device, CFRunLoopGetMain(), CFRunLoopMode.commonModes.rawValue)
            _ = IOHIDDeviceClose(ctx.device, IOOptionBits(kIOHIDOptionsTypeNone))
            ctx.isOpen = false
        }
    }

    private func setupManager() {
        let mgr = IOHIDManagerCreate(kCFAllocatorDefault, IOOptionBits(kIOHIDOptionsTypeNone))
        self.manager = mgr

        let match: [String: Any] = [
            kIOHIDVendorIDKey as String: 0x046d
        ]
        IOHIDManagerSetDeviceMatching(mgr, match as CFDictionary)

        let selfPtr = Unmanaged.passUnretained(self).toOpaque()

        let matchCallback: IOHIDDeviceCallback = { context, _, _, device in
            guard let context = context else { return }
            let driver = Unmanaged<LogitechHapticDriver>.fromOpaque(context).takeUnretainedValue()
            driver.handleDeviceMatched(device)
        }

        let removalCallback: IOHIDDeviceCallback = { context, _, _, device in
            guard let context = context else { return }
            let driver = Unmanaged<LogitechHapticDriver>.fromOpaque(context).takeUnretainedValue()
            driver.handleDeviceRemoved(device)
        }

        IOHIDManagerRegisterDeviceMatchingCallback(mgr, matchCallback, selfPtr)
        IOHIDManagerRegisterDeviceRemovalCallback(mgr, removalCallback, selfPtr)
        IOHIDManagerScheduleWithRunLoop(mgr, CFRunLoopGetMain(), CFRunLoopMode.commonModes.rawValue)
        _ = IOHIDManagerOpen(mgr, IOOptionBits(kIOHIDOptionsTypeNone))

        if let devSet = IOHIDManagerCopyDevices(mgr) as? Set<IOHIDDevice> {
            for dev in devSet {
                handleDeviceMatched(dev)
            }
        }
    }

    private static func intProperty(_ device: IOHIDDevice, key: String) -> Int? {
        guard let ref = IOHIDDeviceGetProperty(device, key as CFString) else { return nil }
        return ref as? Int
    }

    private static func supportsHIDPlusPlusLongOutput(_ device: IOHIDDevice) -> Bool {
        if let maxOut = intProperty(device, key: kIOHIDMaxOutputReportSizeKey as String), maxOut >= 20 {
            return true
        }
        if let page = intProperty(device, key: kIOHIDPrimaryUsagePageKey as String),
           page == 0xFF43 || page == 0xFF00 {
            return true
        }
        if let pairs = IOHIDDeviceGetProperty(device, kIOHIDDeviceUsagePairsKey as CFString) as? [[String: Any]] {
            for pair in pairs {
                if let page = pair[kIOHIDDeviceUsagePageKey as String] as? Int,
                   page == 0xFF43 || page == 0xFF00 {
                    return true
                }
            }
        }
        return false
    }

    private func handleDeviceMatched(_ device: IOHIDDevice) {
        let key = ObjectIdentifier(device)
        if contexts[key] != nil {
            return
        }
        guard Self.supportsHIDPlusPlusLongOutput(device) else {
            return
        }

        let pid = Self.intProperty(device, key: kIOHIDProductIDKey as String) ?? 0
        let ctx = LogitechHIDContext(driver: self, device: device, productID: pid)
        contexts[key] = ctx

        let openRes = IOHIDDeviceOpen(device, IOOptionBits(kIOHIDOptionsTypeNone))
        ctx.isOpen = (openRes == kIOReturnSuccess)
        guard ctx.isOpen else {
            return
        }

        IOHIDDeviceScheduleWithRunLoop(device, CFRunLoopGetMain(), CFRunLoopMode.commonModes.rawValue)

        let ctxPtr = Unmanaged.passUnretained(ctx).toOpaque()
        let reportCallback: IOHIDReportCallback = { context, _, _, _, reportID, report, reportLength in
            guard let context = context else { return }
            let hidCtx = Unmanaged<LogitechHIDContext>.fromOpaque(context).takeUnretainedValue()
            hidCtx.driver?.handleInputReport(context: hidCtx, reportID: reportID, report: report, length: reportLength)
        }

        IOHIDDeviceRegisterInputReportCallback(
            device,
            ctx.reportBuffer,
            ctx.reportBufferCapacity,
            reportCallback,
            ctxPtr
        )

        // Send Root.getFeature(0x19B0) query for direct BLE/USB (0xFF) and Bolt/Unifying receiver slots (0x01..0x06)
        let candidateIndices: [UInt8] = (pid == 0xb042) ? [0xFF] : [0xFF, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06]
        for devIdx in candidateIndices {
            sendRootGetFeatureHaptic(device: device, devIdx: devIdx)
        }
        if pid == 0xb042 {
            sendGetCapabilities(device: device, devIdx: 0xFF, featIdx: 0x0B)
            sendGetHapticConfig(device: device, devIdx: 0xFF, featIdx: 0x0B)
        }
    }

    private func handleDeviceRemoved(_ device: IOHIDDevice) {
        let key = ObjectIdentifier(device)
        guard let ctx = contexts.removeValue(forKey: key) else { return }
        closeContext(ctx)
    }

    private func handleInputReport(
        context: LogitechHIDContext,
        reportID: UInt32,
        report: UnsafeMutablePointer<UInt8>,
        length: CFIndex
    ) {
        if reportID == 0x02 {
            lastMouseReportTimestamp = mach_absolute_time()
        }
        guard length >= 7 else { return }
        let bytes = Array(UnsafeBufferPointer(start: report, count: length))
        guard bytes[0] == 0x11 || bytes[0] == 0x10 else { return }
        let devIdx = bytes[1]
        let featIdx = bytes[2]
        let funcSwId = bytes[3]
        // Root feature (0x00), function 0x00 (getFeature), software ID 0x0A -> 0x0A
        if featIdx == 0x00 && funcSwId == 0x0A {
            let discoveredFeatIdx = bytes[4]
            if discoveredFeatIdx != 0x00 {
                let isNew = (context.featureIndices[devIdx] != discoveredFeatIdx)
                context.featureIndices[devIdx] = discoveredFeatIdx
                if isNew {
                    sendGetCapabilities(device: context.device, devIdx: devIdx, featIdx: discoveredFeatIdx)
                    sendGetHapticConfig(device: context.device, devIdx: devIdx, featIdx: discoveredFeatIdx)
                }
            }
        } else if featIdx != 0x00 && (funcSwId & 0x0F) == 0x0B {
            guard context.featureIndices[devIdx] == featIdx || (devIdx == 0x00 && context.featureIndices[0xFF] == featIdx) else {
                return
            }
            let funcId = (funcSwId >> 4) & 0x0F
            if funcId == 0x00 && length >= 12 {
                context.isContinuousLevel = (bytes[6] == 0x00)
                context.supportsWhisperCollision = (bytes[8] & 0x08) != 0
            } else if (funcId == 0x01 || funcId == 0x02) && length >= 6 {
                let flags = bytes[4]
                let level = bytes[5]
                context.powerSaveFlagsByDev[devIdx] = flags & 0x02
                if (flags & 0x01) != 0 {
                    context.lastConfiguredLevel[devIdx] = level
                    if devIdx == 0xFF {
                        context.lastConfiguredLevel[0x00] = level
                    }
                }
            }
        }
    }

    private func sendHIDPP20Report(device: IOHIDDevice, payload: [UInt8]) {
        var packet = [UInt8](repeating: 0, count: 20)
        let count = min(payload.count, 20)
        for i in 0..<count {
            packet[i] = payload[i]
        }
        packet[0] = 0x11
        _ = packet.withUnsafeBufferPointer { ptr -> IOReturn in
            guard let base = ptr.baseAddress else { return kIOReturnBadArgument }
            return IOHIDDeviceSetReport(device, kIOHIDReportTypeOutput, CFIndex(0x11), base, 20)
        }
    }

    private func sendRootGetFeatureHaptic(device: IOHIDDevice, devIdx: UInt8) {
        sendHIDPP20Report(device: device, payload: [0x11, devIdx, 0x00, 0x0A, 0x19, 0xB0, 0x00])
    }

    private func sendGetCapabilities(device: IOHIDDevice, devIdx: UInt8, featIdx: UInt8) {
        sendHIDPP20Report(device: device, payload: [0x11, devIdx, featIdx, 0x0B, 0x00, 0x00, 0x00])
    }

    private func sendGetHapticConfig(device: IOHIDDevice, devIdx: UInt8, featIdx: UInt8) {
        sendHIDPP20Report(device: device, payload: [0x11, devIdx, featIdx, 0x1B, 0x00, 0x00, 0x00])
    }

    private static func waveformId(for kind: String, intensity: HapticIntensity, supportsWhisper: Bool) -> UInt8? {
        switch kind.lowercased() {
        case "tick", "alignment":
            switch intensity {
            case .light: return supportsWhisper ? 0x1B : 0x04 // WHISPER_COLLISION / SUBTLE_COLLISION
            case .medium, .firm: return 0x04                  // SUBTLE_COLLISION (Logi Actions Ring hover_on_action)
            case .strong: return 0x02                         // SHARP_COLLISION
            }
        case "level":
            switch intensity {
            case .light: return 0x04          // SUBTLE_COLLISION
            case .medium, .firm: return 0x03  // DAMP_COLLISION
            case .strong: return 0x02         // SHARP_COLLISION
            }
        case "generic":
            switch intensity {
            case .light, .medium, .firm: return 0x01 // DAMP_STATE_CHANGE (Logi Actions Ring button_action_run)
            case .strong: return 0x00                // SHARP_STATE_CHANGE
            }
        case "success":
            return 0x07 // COMPLETED
        default:
            return nil
        }
    }

    func play(kind: String, intensity: HapticIntensity) {
        let normKind = kind.lowercased()
        if normKind == "tick" || normKind == "alignment" {
            let now = ProcessInfo.processInfo.systemUptime
            if now - lastTickPlayUptime < tickCooldownSeconds { return }
            lastTickPlayUptime = now
        }

        if contexts.isEmpty || contexts.values.allSatisfy({ !$0.isOpen || $0.featureIndices.isEmpty }) {
            let now = ProcessInfo.processInfo.systemUptime
            if now - lastReopenAttemptUptime >= reopenCooldownSeconds {
                reopenManager()
            }
        }

        for ctx in contexts.values {
            guard ctx.isOpen else { continue }
            if ctx.featureIndices.isEmpty {
                let candidateIndices: [UInt8] = (ctx.productID == 0xb042) ? [0xFF] : [0xFF, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06]
                for devIdx in candidateIndices {
                    sendRootGetFeatureHaptic(device: ctx.device, devIdx: devIdx)
                }
                continue
            }

            guard let waveform = Self.waveformId(
                for: normKind,
                intensity: intensity,
                supportsWhisper: ctx.supportsWhisperCollision
            ) else {
                continue
            }

            let targets: [(UInt8, UInt8)]
            if let ffIdx = ctx.featureIndices[0xFF] {
                var list: [(UInt8, UInt8)] = [(0xFF, ffIdx)]
                for (devIdx, featIdx) in ctx.featureIndices where devIdx != 0xFF && devIdx != 0x00 {
                    list.append((devIdx, featIdx))
                }
                targets = list
            } else {
                targets = Array(ctx.featureIndices)
            }

            let levelByte = ctx.isContinuousLevel ? intensity.logitechContinuousLevelByte : intensity.logitechDiscreteLevelByte
            for (devIdx, featIdx) in targets {
                if ctx.lastConfiguredLevel[devIdx] != levelByte {
                    let powerSaveBit = ctx.powerSaveFlagsByDev[devIdx] ?? 0x00
                    sendHIDPP20Report(
                        device: ctx.device,
                        payload: [0x11, devIdx, featIdx, 0x2B, 0x01 | powerSaveBit, levelByte, 0x00]
                    )
                    ctx.lastConfiguredLevel[devIdx] = levelByte
                    ctx.deferredPlayWorkItem[devIdx]?.cancel()
                    let dev = ctx.device
                    let item = DispatchWorkItem { [weak self] in
                        self?.sendHIDPP20Report(
                            device: dev,
                            payload: [0x11, devIdx, featIdx, 0x4B, waveform, 0x00, 0x00]
                        )
                    }
                    ctx.deferredPlayWorkItem[devIdx] = item
                    DispatchQueue.main.asyncAfter(deadline: .now() + .milliseconds(20), execute: item)
                } else {
                    ctx.deferredPlayWorkItem[devIdx]?.cancel()
                    sendHIDPP20Report(
                        device: ctx.device,
                        payload: [0x11, devIdx, featIdx, 0x4B, waveform, 0x00, 0x00]
                    )
                }
            }
        }
    }
}

final class FeedbackPlayer {
    private static let variantRatios: [Double] = [0.986, 1.000, 1.012, 1.022]
    private static let variantGainDb: [Float] = [-0.35, 0.05, 0.35, -0.20]
    private static let voiceCount = 4
    private static let fxVoiceCount = 2
    private static let nsSoundFallbackVoices = 3
    private static let idleTimeoutSeconds: TimeInterval = 3.0
    private static let minAcousticIntervalSeconds: CFTimeInterval = 0.022
    private static let rapidScrubSoftenSeconds: CFTimeInterval = 0.060
    private static let rapidScrubGainMultiplier: Float = 0.78

    private let engine = AVAudioEngine()
    private var playerNodes: [AVAudioPlayerNode] = []
    private var fxPlayerNodes: [AVAudioPlayerNode] = []
    private var fxNodeFormats: [AVAudioFormat?] = []
    private var variantBuffers: [AVAudioPCMBuffer] = []
    private var errorBuffer: AVAudioPCMBuffer?
    private var successBuffer: AVAudioPCMBuffer?
    private var bufferFormat: AVAudioFormat?
    private var engineGraphBuilt = false
    private var engineNeedsRebuild = false
    private var configObserver: NSObjectProtocol?
    private var idleStopWorkItem: DispatchWorkItem?
    private var nextVoiceIndex = 0
    private var nextFxVoiceIndex = 0
    private var lastBufferIndex = -1
    private var lastTickTimestamp: CFTimeInterval = 0

    private var nsSoundPool: [NSSound] = []
    private var errorFallbackSound: NSSound?
    private var successFallbackSound: NSSound?
    private var nextNSSoundIndex = 0
    private var loadedWavPath: String?
    private var intensity: HapticIntensity = .firm

    init(wavPath: String? = nil, intensity: String? = nil) {
        if let intensity = intensity {
            self.intensity = HapticIntensity.parse(intensity)
        }
        installEngineConfigurationObserver()
        if let wavPath = wavPath, FileManager.default.fileExists(atPath: wavPath) {
            loadAudioAssets(from: wavPath)
        }
        _ = LogitechHapticDriver.shared
    }

    deinit {
        idleStopWorkItem?.cancel()
        if let observer = configObserver {
            NotificationCenter.default.removeObserver(observer)
        }
        for node in playerNodes {
            node.stop()
        }
        for node in fxPlayerNodes {
            node.stop()
        }
        if engine.isRunning {
            engine.stop()
        }
    }

    func updateWavPath(_ path: String?) {
        guard let path = path, path != loadedWavPath, FileManager.default.fileExists(atPath: path) else { return }
        loadAudioAssets(from: path)
    }

    func updateIntensity(_ raw: String?) {
        guard let raw = raw else { return }
        intensity = HapticIntensity.parse(raw)
    }

    func warmUp() {
        guard !variantBuffers.isEmpty || errorBuffer != nil || successBuffer != nil else { return }
        _ = ensureEngineRunning()
    }

    private func installEngineConfigurationObserver() {
        configObserver = NotificationCenter.default.addObserver(
            forName: .AVAudioEngineConfigurationChange,
            object: engine,
            queue: .main
        ) { [weak self] _ in
            guard let self = self else { return }
            self.idleStopWorkItem?.cancel()
            self.idleStopWorkItem = nil
            self.engineNeedsRebuild = true
        }
    }

    private func loadAudioAssets(from path: String) {
        loadedWavPath = path

        var fallbackPool: [NSSound] = []
        fallbackPool.reserveCapacity(Self.nsSoundFallbackVoices)
        for _ in 0..<Self.nsSoundFallbackVoices {
            if let s = NSSound(contentsOfFile: path, byReference: false) {
                fallbackPool.append(s)
            }
        }
        nsSoundPool = fallbackPool
        nextNSSoundIndex = 0

        let baseDir = URL(fileURLWithPath: path).deletingLastPathComponent()
        let errorPath = baseDir.appendingPathComponent("flip-error.wav").path
        let successPath = baseDir.appendingPathComponent("flip-success.wav").path

        if FileManager.default.fileExists(atPath: errorPath) {
            errorBuffer = decodeFloat32Buffer(from: errorPath)
            errorFallbackSound = NSSound(contentsOfFile: errorPath, byReference: false)
        } else {
            errorBuffer = nil
            errorFallbackSound = nil
        }

        if FileManager.default.fileExists(atPath: successPath) {
            successBuffer = decodeFloat32Buffer(from: successPath)
            successFallbackSound = NSSound(contentsOfFile: successPath, byReference: false)
        } else {
            successBuffer = nil
            successFallbackSound = nil
        }

        if let sourceBuffer = decodeFloat32Buffer(from: path) {
            var buffers: [AVAudioPCMBuffer] = []
            buffers.reserveCapacity(Self.variantRatios.count)
            for (ratio, db) in zip(Self.variantRatios, Self.variantGainDb) {
                if let buf = resampleTickBuffer(
                    sourceBuffer,
                    speedRatio: ratio,
                    gainDb: db
                ) {
                    buffers.append(buf)
                }
            }
            variantBuffers = buffers
            bufferFormat = sourceBuffer.format
        } else {
            variantBuffers = []
            bufferFormat = nil
        }

        lastBufferIndex = -1
        nextVoiceIndex = 0
        nextFxVoiceIndex = 0
        engineNeedsRebuild = true
    }

    private func decodeFloat32Buffer(from path: String) -> AVAudioPCMBuffer? {
        let url = URL(fileURLWithPath: path)
        guard let audioFile = try? AVAudioFile(forReading: url) else { return nil }
        let fileFormat = audioFile.processingFormat
        let frameCount = AVAudioFrameCount(audioFile.length)
        guard frameCount > 0,
              let readBuffer = AVAudioPCMBuffer(pcmFormat: fileFormat, frameCapacity: frameCount) else {
            return nil
        }
        do {
            try audioFile.read(into: readBuffer)
        } catch {
            return nil
        }

        let n = Int(readBuffer.frameLength)
        let channels = Int(fileFormat.channelCount)
        guard n > 0, channels > 0, let srcData = readBuffer.floatChannelData else { return nil }

        guard let outFormat = AVAudioFormat(
            commonFormat: .pcmFormatFloat32,
            sampleRate: fileFormat.sampleRate,
            channels: fileFormat.channelCount,
            interleaved: false
        ),
        let outBuffer = AVAudioPCMBuffer(pcmFormat: outFormat, frameCapacity: AVAudioFrameCount(n)),
        let dstData = outBuffer.floatChannelData else {
            return nil
        }
        outBuffer.frameLength = AVAudioFrameCount(n)

        let byteCount = n * MemoryLayout<Float>.size
        for ch in 0..<channels {
            memcpy(dstData[ch], srcData[ch], byteCount)
        }
        return outBuffer
    }

    private func resampleTickBuffer(
        _ source: AVAudioPCMBuffer,
        speedRatio: Double,
        gainDb: Float
    ) -> AVAudioPCMBuffer? {
        let srcCount = Int(source.frameLength)
        let channels = Int(source.format.channelCount)
        guard srcCount > 1, channels > 0, let srcData = source.floatChannelData else { return nil }
        let outCount = max(2, Int((Double(srcCount) / speedRatio).rounded()))
        guard let buffer = AVAudioPCMBuffer(
            pcmFormat: source.format,
            frameCapacity: AVAudioFrameCount(outCount)
        ) else {
            return nil
        }
        buffer.frameLength = AVAudioFrameCount(outCount)
        guard let dstData = buffer.floatChannelData else { return nil }

        let linearGain = powf(10.0, gainDb / 20.0)
        for ch in 0..<channels {
            let src = srcData[ch]
            let dst = dstData[ch]
            @inline(__always)
            func sampleAt(_ idx: Int) -> Float {
                (idx >= 0 && idx < srcCount) ? src[idx] : 0.0
            }

            for i in 0..<outCount {
                if i == 0 || i == outCount - 1 {
                    dst[i] = 0.0
                    continue
                }
                let pos = Double(i) * speedRatio
                let idx = Int(floor(pos))
                let t = Float(pos - Double(idx))
                let y0 = sampleAt(idx - 1)
                let y1 = sampleAt(idx)
                let y2 = sampleAt(idx + 1)
                let y3 = sampleAt(idx + 2)

                let c0 = y1
                let c1 = 0.5 * (y2 - y0)
                let c2 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3
                let c3 = 0.5 * (y3 - y0) + 1.5 * (y1 - y2)
                let interp = ((c3 * t + c2) * t + c1) * t + c0
                dst[i] = max(-1.0, min(1.0, interp * linearGain))
            }
        }
        return buffer
    }

    private func tearDownEngineGraph() {
        for node in playerNodes {
            node.stop()
            engine.detach(node)
        }
        playerNodes.removeAll()
        for node in fxPlayerNodes {
            node.stop()
            engine.detach(node)
        }
        fxPlayerNodes.removeAll()
        fxNodeFormats.removeAll()
        if engine.isRunning {
            engine.stop()
        }
        engine.reset()
        engineGraphBuilt = false
    }

    private func ensureEngineRunning() -> Bool {
        let defaultFxFormat = errorBuffer?.format ?? successBuffer?.format ?? bufferFormat
        guard bufferFormat != nil || defaultFxFormat != nil else { return false }

        if engineNeedsRebuild || !engineGraphBuilt {
            tearDownEngineGraph()
            if !variantBuffers.isEmpty, let tickFormat = bufferFormat {
                var nodes: [AVAudioPlayerNode] = []
                nodes.reserveCapacity(Self.voiceCount)
                for _ in 0..<Self.voiceCount {
                    let node = AVAudioPlayerNode()
                    engine.attach(node)
                    engine.connect(node, to: engine.mainMixerNode, format: tickFormat)
                    nodes.append(node)
                }
                playerNodes = nodes
            }
            if let fxFormat = defaultFxFormat {
                var fxNodes: [AVAudioPlayerNode] = []
                fxNodes.reserveCapacity(Self.fxVoiceCount)
                for _ in 0..<Self.fxVoiceCount {
                    let node = AVAudioPlayerNode()
                    engine.attach(node)
                    engine.connect(node, to: engine.mainMixerNode, format: fxFormat)
                    fxNodes.append(node)
                }
                fxPlayerNodes = fxNodes
                fxNodeFormats = Array(repeating: fxFormat, count: Self.fxVoiceCount)
            }
            nextVoiceIndex = 0
            nextFxVoiceIndex = 0
            engineGraphBuilt = true
            engineNeedsRebuild = false
        }

        if !engine.isRunning {
            engine.prepare()
            do {
                try engine.start()
            } catch {
                engineNeedsRebuild = true
                return false
            }
        }

        scheduleIdleShutdown()
        return true
    }

    private func scheduleIdleShutdown() {
        idleStopWorkItem?.cancel()
        let item = DispatchWorkItem { [weak self] in
            self?.stopEngineForIdle()
        }
        idleStopWorkItem = item
        DispatchQueue.main.asyncAfter(deadline: .now() + Self.idleTimeoutSeconds, execute: item)
    }

    private func stopEngineForIdle() {
        idleStopWorkItem = nil
        for node in playerNodes {
            node.stop()
        }
        for node in fxPlayerNodes {
            node.stop()
        }
        if engine.isRunning {
            engine.stop()
        }
    }

    private func nextVariantBuffer() -> AVAudioPCMBuffer? {
        let count = variantBuffers.count
        guard count > 0 else { return nil }
        let idx: Int
        if count == 1 {
            idx = 0
        } else if lastBufferIndex < 0 {
            idx = Int.random(in: 0..<count)
        } else {
            let step = Int.random(in: 1..<count)
            idx = (lastBufferIndex + step) % count
        }
        lastBufferIndex = idx
        return variantBuffers[idx]
    }

    private func playFxBuffer(_ buffer: AVAudioPCMBuffer, gain: Float) -> Bool {
        guard ensureEngineRunning(), !fxPlayerNodes.isEmpty else { return false }
        let idx = nextFxVoiceIndex
        nextFxVoiceIndex = (nextFxVoiceIndex + 1) % fxPlayerNodes.count
        let node = fxPlayerNodes[idx]
        if fxNodeFormats[idx] != buffer.format {
            node.stop()
            engine.disconnectNodeOutput(node)
            engine.connect(node, to: engine.mainMixerNode, format: buffer.format)
            fxNodeFormats[idx] = buffer.format
        }
        node.volume = gain
        node.scheduleBuffer(buffer, at: nil, options: [], completionHandler: nil)
        if !node.isPlaying {
            node.play()
        }
        scheduleIdleShutdown()
        return true
    }

    private func playSoundSuccess(intensity: HapticIntensity) {
        let gain = intensity.audioFxGain
        if let buffer = successBuffer, playFxBuffer(buffer, gain: gain) {
            return
        }
        if let sound = successFallbackSound {
            if sound.isPlaying {
                sound.stop()
            }
            sound.volume = gain
            sound.currentTime = 0
            sound.play()
        }
    }

    private func playSoundError(intensity: HapticIntensity) {
        let gain = intensity.audioFxGain
        if let buffer = errorBuffer, playFxBuffer(buffer, gain: gain) {
            return
        }
        if let sound = errorFallbackSound {
            if sound.isPlaying {
                sound.stop()
            }
            sound.volume = gain
            sound.currentTime = 0
            sound.play()
        } else {
            NSSound.beep()
        }
    }

    private func playSoundTick(intensity: HapticIntensity) {
        let now = CACurrentMediaTime()
        if lastTickTimestamp > 0 {
            let elapsed = now - lastTickTimestamp
            if elapsed < Self.minAcousticIntervalSeconds {
                return
            }
        }

        var gain = intensity.audioTickGain
        if lastTickTimestamp > 0 && (now - lastTickTimestamp) < Self.rapidScrubSoftenSeconds {
            gain *= Self.rapidScrubGainMultiplier
        }
        lastTickTimestamp = now

        if !variantBuffers.isEmpty && ensureEngineRunning(),
           let buffer = nextVariantBuffer(),
           !playerNodes.isEmpty {
            let node = playerNodes[nextVoiceIndex]
            nextVoiceIndex = (nextVoiceIndex + 1) % playerNodes.count
            node.volume = gain
            node.scheduleBuffer(buffer, at: nil, options: [], completionHandler: nil)
            if !node.isPlaying {
                node.play()
            }
            scheduleIdleShutdown()
            return
        }

        if !nsSoundPool.isEmpty {
            let sound = nsSoundPool[nextNSSoundIndex]
            nextNSSoundIndex = (nextNSSoundIndex + 1) % nsSoundPool.count
            if sound.isPlaying {
                sound.stop()
            }
            sound.volume = gain
            sound.currentTime = 0
            sound.play()
        } else {
            AudioServicesPlaySystemSound(1104)
        }
    }

    private func performActiveDeviceHaptic(kind: String, intensity: HapticIntensity) {
        let source = ActivePointerDetector.shared.currentSource(
            logitechDirectReportTimestamp: LogitechHapticDriver.shared.lastMouseReportTimestamp
        )
        switch source {
        case .trackpad:
            performTrackpadHaptic(kind: kind, intensity: intensity)
        case .logitechMouse:
            LogitechHapticDriver.shared.play(kind: kind, intensity: intensity)
        case .otherMouse:
            break
        case .unknown:
            performTrackpadHaptic(kind: kind, intensity: intensity)
        }
    }

    private func performTrackpadHaptic(kind: String, intensity: HapticIntensity) {
        let performer = NSHapticFeedbackManager.defaultPerformer
        switch intensity {
        case .light:
            switch kind {
            case "tick", "alignment":
                performer.perform(.alignment, performanceTime: .now)
            case "level":
                performer.perform(.levelChange, performanceTime: .now)
            case "generic", "success":
                performer.perform(.generic, performanceTime: .now)
            default:
                break
            }
        case .medium:
            switch kind {
            case "tick", "alignment":
                performer.perform(.levelChange, performanceTime: .now)
            case "level", "generic":
                performer.perform(.generic, performanceTime: .now)
            case "success":
                performer.perform(.levelChange, performanceTime: .now)
                DispatchQueue.main.asyncAfter(deadline: .now() + .milliseconds(60)) {
                    NSHapticFeedbackManager.defaultPerformer.perform(.generic, performanceTime: .now)
                }
            default:
                break
            }
        case .firm:
            switch kind {
            case "tick", "alignment":
                performer.perform(.generic, performanceTime: .now)
            case "level":
                performer.perform(.generic, performanceTime: .now)
                DispatchQueue.main.asyncAfter(deadline: .now() + .milliseconds(10)) {
                    NSHapticFeedbackManager.defaultPerformer.perform(.levelChange, performanceTime: .now)
                }
            case "generic":
                performer.perform(.generic, performanceTime: .now)
                DispatchQueue.main.asyncAfter(deadline: .now() + .milliseconds(10)) {
                    NSHapticFeedbackManager.defaultPerformer.perform(.alignment, performanceTime: .now)
                }
            case "success":
                performer.perform(.levelChange, performanceTime: .now)
                DispatchQueue.main.asyncAfter(deadline: .now() + .milliseconds(60)) {
                    NSHapticFeedbackManager.defaultPerformer.perform(.generic, performanceTime: .now)
                }
            default:
                break
            }
        case .strong:
            switch kind {
            case "tick", "alignment":
                performer.perform(.generic, performanceTime: .now)
                DispatchQueue.main.asyncAfter(deadline: .now() + .milliseconds(10)) {
                    NSHapticFeedbackManager.defaultPerformer.perform(.levelChange, performanceTime: .now)
                }
            case "level":
                performer.perform(.generic, performanceTime: .now)
                DispatchQueue.main.asyncAfter(deadline: .now() + .milliseconds(12)) {
                    NSHapticFeedbackManager.defaultPerformer.perform(.generic, performanceTime: .now)
                }
            case "generic":
                performer.perform(.generic, performanceTime: .now)
                DispatchQueue.main.asyncAfter(deadline: .now() + .milliseconds(10)) {
                    NSHapticFeedbackManager.defaultPerformer.perform(.generic, performanceTime: .now)
                }
            case "success":
                performer.perform(.generic, performanceTime: .now)
                DispatchQueue.main.asyncAfter(deadline: .now() + .milliseconds(12)) {
                    NSHapticFeedbackManager.defaultPerformer.perform(.generic, performanceTime: .now)
                }
                DispatchQueue.main.asyncAfter(deadline: .now() + .milliseconds(70)) {
                    NSHapticFeedbackManager.defaultPerformer.perform(.generic, performanceTime: .now)
                }
            default:
                break
            }
        }
    }

    func play(kind: String, intensityOverride: String? = nil) {
        let effIntensity = intensityOverride != nil ? HapticIntensity.parse(intensityOverride) : self.intensity
        let normalizedKind = kind.lowercased()
        switch normalizedKind {
        case "tick":
            playSoundTick(intensity: effIntensity)
            performActiveDeviceHaptic(kind: "tick", intensity: effIntensity)
        case "sound":
            playSoundTick(intensity: effIntensity)
        case "success":
            playSoundSuccess(intensity: effIntensity)
            performActiveDeviceHaptic(kind: "success", intensity: effIntensity)
        case "success_sound":
            playSoundSuccess(intensity: effIntensity)
        case "success_haptic":
            performActiveDeviceHaptic(kind: "success", intensity: effIntensity)
        case "error":
            playSoundError(intensity: effIntensity)
        case "alignment", "level", "generic":
            performActiveDeviceHaptic(kind: normalizedKind, intensity: effIntensity)
        default:
            break
        }
    }
}

final class MonitorController {
    static let shared = MonitorController()

    private var dragKeyCode: UInt16 = 125 // Down Arrow
    private var dragModifierOnly: String? = nil
    private var idleShortcut: ParsedShortcut? = ParsedShortcut.parse("Option+Shift+Space")
    private var mouseTriggerButton: Int64? = nil // e.g. 2 = middle, 3/4 = side buttons
    private var wheelOpen = false

    private let dragPasteboard = NSPasteboard(name: .drag)
    private var lastChangeCount: Int = -1
    private var pendingChangeCount: Int = -1
    private var pendingEmptyRetries: Int = 0
    private var pendingEmptyStart: TimeInterval = 0
    private var dragActive = false
    private var dragFiles: [String] = []
    private var dragWheelTriggeredForDrag = false
    private var dragKeyHeld = false
    private var dragKeyConsumed = false
    private var rawToggleKeyConsumed = false
    private var faceToggleKeyConsumed = false
    private var spaceToggleKeyConsumed = false
    private var settingsShortcutConsumed = false
    private var mouseTriggerHeld = false
    private var lastEmittedX: Double = -9999
    private var lastEmittedY: Double = -9999

    private enum EventTapMode: String {
        case none = "none"
        case listenOnly = "listen_only"
        case defaultTap = "default_tap"
    }

    private static let carbonHotKeySignature: OSType = 0x464C4950 // 'FLIP'
    private static let carbonHotKeyID: UInt32 = 1

    private var eventTap: CFMachPort?
    private var runLoopSource: CFRunLoopSource?
    private var fallbackMonitor: Any?
    private var activeTapMode: EventTapMode = .none
    private var currentAxTrusted = false
    private var currentListenTrusted = false
    private var carbonHotKeyHandlerRef: EventHandlerRef?
    private var carbonHotKeyRef: EventHotKeyRef?
    private var monitorsInitialized = false
    private var axPollTick = 0
    private var dragTimer: Timer?
    private var feedback = FeedbackPlayer()

    func configure(dragKey: String?, idleShortcutSpec: String?, mouseButton: Int?, wavPath: String?, isWheelOpen: Bool?, hapticIntensity: String? = nil, auraEnabled: Bool? = nil) {
        let previousIdleShortcut = idleShortcut
        if let dk = dragKey {
            dragKeyHeld = false
            dragKeyConsumed = false
            let low = dk.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
            if ["option", "alt", "control", "ctrl", "shift", "command", "cmd", "fn", "function", "globe"].contains(low) {
                dragModifierOnly = (low == "function" || low == "globe") ? "fn" : low
                if dragModifierOnly == "fn" {
                    dragKeyCode = 63
                }
            } else {
                dragModifierOnly = nil
                if let kc = keyCodeForName(dk) {
                    dragKeyCode = kc
                }
            }
        }
        if let idle = idleShortcutSpec {
            idleShortcut = ParsedShortcut.parse(idle)
        }
        if let mb = mouseButton {
            mouseTriggerButton = mb >= 2 ? Int64(mb) : nil
            if mouseTriggerButton == nil {
                mouseTriggerHeld = false
            }
        }
        if let wp = wavPath {
            feedback.updateWavPath(wp)
        }
        if let hi = hapticIntensity {
            feedback.updateIntensity(hi)
        }
        if let aura = auraEnabled {
            AuraOverlayController.shared.isEnabled = aura
        }
        if let open = isWheelOpen {
            wheelOpen = open
        }
        if monitorsInitialized && idleShortcut != previousIdleShortcut {
            syncCarbonHotKey()
        }
    }

    func run(dragKey: String, idleShortcutSpec: String, mouseButton: Int?, wavPath: String?, hapticIntensity: String? = nil, auraEnabled: Bool? = nil) -> Never {
        let app = NSApplication.shared
        app.setActivationPolicy(.prohibited)

        configure(dragKey: dragKey, idleShortcutSpec: idleShortcutSpec, mouseButton: mouseButton, wavPath: wavPath, isWheelOpen: false, hapticIntensity: hapticIntensity, auraEnabled: auraEnabled)
        lastChangeCount = dragPasteboard.changeCount

        let axTrusted = AXIsProcessTrusted()
        let listenTrusted = CGPreflightListenEventAccess()
        setupInputMonitors(axTrusted: axTrusted, listenTrusted: listenTrusted)
        setupDragPolling()
        setupStdinReader()

        writeJSON([
            "event": "ready",
            "axTrusted": currentAxTrusted,
            "listenTrusted": currentListenTrusted,
            "tapMode": activeTapMode.rawValue,
            "dragKeyCode": Int(dragKeyCode)
        ])

        app.run()
        exit(0)
    }

    private func setupStdinReader() {
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            while let line = readLine() {
                let trimmed = line.trimmingCharacters(in: .whitespacesAndNewlines)
                if trimmed.isEmpty { continue }
                guard let data = trimmed.data(using: .utf8),
                      let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                      let cmd = json["cmd"] as? String else {
                    continue
                }
                DispatchQueue.main.async {
                    guard let self = self else { return }
                    switch cmd {
                    case "set_config":
                        let mb: Int?
                        if json.keys.contains("mouseButton") {
                            mb = (json["mouseButton"] as? Int) ?? -1
                        } else {
                            mb = nil
                        }
                        self.configure(
                            dragKey: json["dragKey"] as? String,
                            idleShortcutSpec: json["idleShortcut"] as? String,
                            mouseButton: mb,
                            wavPath: json["wavPath"] as? String,
                            isWheelOpen: json["wheelOpen"] as? Bool,
                            hapticIntensity: json["hapticIntensity"] as? String,
                            auraEnabled: json["auraEnabled"] as? Bool
                        )
                    case "wheel_state":
                        let nextOpen = (json["open"] as? Bool) ?? false
                        self.wheelOpen = nextOpen
                        if !nextOpen {
                            self.dragWheelTriggeredForDrag = false
                            self.rawToggleKeyConsumed = false
                            self.faceToggleKeyConsumed = false
                            self.spaceToggleKeyConsumed = false
                            self.settingsShortcutConsumed = false
                            self.lastEmittedX = -9999
                            self.lastEmittedY = -9999
                            AuraOverlayController.shared.hide(force: false)
                        } else {
                            self.feedback.warmUp()
                            self.emitMouseMoveIfChanged(force: true)
                            if AuraOverlayController.shared.isEnabled {
                                AuraOverlayController.shared.showActive()
                            }
                        }
                    case "aura":
                        let state = (json["state"] as? String) ?? "off"
                        switch state {
                        case "active":
                            self.feedback.warmUp()
                            AuraOverlayController.shared.showActive()
                        case "success":
                            AuraOverlayController.shared.showSuccess()
                        case "off", "hide":
                            AuraOverlayController.shared.hide(force: true)
                        default:
                            break
                        }
                    case "feedback":
                        if let kind = json["kind"] as? String {
                            let intensity = json["intensity"] as? String
                            self.feedback.play(kind: kind, intensityOverride: intensity)
                        }
                    case "recheck_permissions":
                        self.refreshAccessibilityMonitor(emitChange: true)
                    case "quit":
                        exit(0)
                    default:
                        break
                    }
                }
            }
            // Parent closed stdin -> exit monitor cleanly
            exit(0)
        }
    }

    private func emitMouseMoveIfChanged(force: Bool = false) {
        let pos = currentCursorAndScreen()
        if force || hypot(pos.x - lastEmittedX, pos.y - lastEmittedY) >= 0.4 {
            lastEmittedX = pos.x
            lastEmittedY = pos.y
            writeJSON(["event": "mouse_move", "x": pos.x, "y": pos.y])
        }
    }

    private func triggerDragWheelIfNeeded(files: [String]) {
        guard !dragWheelTriggeredForDrag, !files.isEmpty else { return }
        dragWheelTriggeredForDrag = true
        wheelOpen = true
        feedback.warmUp()
        AuraOverlayController.shared.showActive()
        let pos = currentCursorAndScreen()
        lastEmittedX = pos.x
        lastEmittedY = pos.y
        writeJSON([
            "event": "trigger_drag_wheel",
            "files": files,
            "x": pos.x,
            "y": pos.y,
            "screen": pos.screen.toDict()
        ])
        emitMouseMoveIfChanged(force: true)
    }

    private func setupDragPolling() {
        // Lightweight timer + event-driven drag detection (~60Hz when active).
        // When no mouse button is pressed and drag is inactive, checks changeCount in O(1).
        dragTimer = Timer.scheduledTimer(withTimeInterval: 0.016, repeats: true) { [weak self] _ in
            guard let self = self else { return }
            self.checkDragState()
            if self.wheelOpen || self.mouseTriggerHeld {
                self.emitMouseMoveIfChanged()
            }
            self.axPollTick += 1
            if self.axPollTick >= 90 {
                self.axPollTick = 0
                self.refreshAccessibilityMonitor(emitChange: true)
            }
        }
        RunLoop.main.add(dragTimer!, forMode: .common)

        NSEvent.addGlobalMonitorForEvents(matching: [.leftMouseDragged, .leftMouseUp, .mouseMoved, .otherMouseDragged]) { [weak self] event in
            guard let self = self else { return }
            if event.type == .leftMouseUp {
                self.checkDragState(forceEndIfUp: true)
            } else if event.type == .leftMouseDragged {
                self.checkDragState()
                if self.wheelOpen || self.mouseTriggerHeld {
                    self.emitMouseMoveIfChanged(force: true)
                }
            } else if (event.type == .mouseMoved || event.type == .otherMouseDragged) && (self.wheelOpen || self.mouseTriggerHeld) {
                self.emitMouseMoveIfChanged(force: true)
            }
        }
    }

    private func readDragFileURLs() -> [String] {
        var candidateURLs: [URL] = []
        let options: [NSPasteboard.ReadingOptionKey: Any] = [
            .urlReadingFileURLsOnly: true
        ]
        if let urls = dragPasteboard.readObjects(forClasses: [NSURL.self], options: options) as? [URL], !urls.isEmpty {
            candidateURLs = urls
        } else if let names = dragPasteboard.propertyList(forType: NSPasteboard.PasteboardType("NSFilenamesPboardType")) as? [String], !names.isEmpty {
            candidateURLs = names.map { URL(fileURLWithPath: $0) }
        } else if let items = dragPasteboard.pasteboardItems, !items.isEmpty {
            candidateURLs = items.compactMap { item in
                guard let str = item.string(forType: .fileURL) else { return nil }
                return URL(string: str)
            }
        }

        if candidateURLs.isEmpty || candidateURLs.count > 128 {
            return []
        }

        var seen = Set<String>()
        var result: [String] = []
        result.reserveCapacity(candidateURLs.count)

        for url in candidateURLs {
            let nsURL = url as NSURL
            let resolvedURL = nsURL.isFileReferenceURL() ? (nsURL.filePathURL ?? url) : url
            guard resolvedURL.isFileURL else { continue }
            let path = resolvedURL.path
            guard !path.isEmpty, FileManager.default.fileExists(atPath: path) else { continue }

            let rv = try? resolvedURL.resourceValues(forKeys: [
                .isDirectoryKey,
                .isPackageKey,
                .isSymbolicLinkKey,
                .isAliasFileKey,
                .volumeIsLocalKey
            ])
            if rv?.isSymbolicLink == true || rv?.isAliasFile == true {
                continue
            }
            if rv?.volumeIsLocal == false {
                continue
            }
            let isDir = rv?.isDirectory ?? false
            let isPkg = (rv?.isPackage ?? false) || NSWorkspace.shared.isFilePackage(atPath: path)
            let ext = resolvedURL.pathExtension.lowercased()
            if isDir && !(isPkg && ["pages", "numbers", "key"].contains(ext)) {
                continue
            }
            let stdPath = (path as NSString).standardizingPath
            if seen.insert(stdPath).inserted {
                result.append(stdPath)
            }
            if result.count > 128 {
                return []
            }
        }
        return result
    }

    private func checkDragState(forceEndIfUp: Bool = false, forceReadPasteboard: Bool = false) {
        let leftDown = (NSEvent.pressedMouseButtons & 1) != 0
        let count = dragPasteboard.changeCount

        if leftDown && !forceEndIfUp {
            let shouldRead = (count != lastChangeCount) || (forceReadPasteboard && pendingChangeCount == count)
            if shouldRead {
                let files = readDragFileURLs()
                if !files.isEmpty {
                    lastChangeCount = count
                    pendingChangeCount = -1
                    pendingEmptyRetries = 0
                    pendingEmptyStart = 0

                    let changed = !dragActive || files != dragFiles
                    dragActive = true
                    dragFiles = files
                    if changed {
                        dragWheelTriggeredForDrag = false
                        let pos = currentCursorAndScreen()
                        writeJSON([
                            "event": "drag_start",
                            "files": files,
                            "x": pos.x,
                            "y": pos.y,
                            "screen": pos.screen.toDict()
                        ])
                        if dragKeyHeld && !dragKeyConsumed {
                            dragKeyConsumed = true
                            triggerDragWheelIfNeeded(files: files)
                        }
                    }
                } else {
                    if dragActive {
                        dragActive = false
                        dragFiles = []
                        dragWheelTriggeredForDrag = false
                    }
                    // Defer committing lastChangeCount so 120Hz drag events don't lock out
                    // Finder's ~400ms async pasteboard population window.
                    let now = ProcessInfo.processInfo.systemUptime
                    if pendingChangeCount != count {
                        pendingChangeCount = count
                        pendingEmptyRetries = 1
                        pendingEmptyStart = now
                    } else {
                        pendingEmptyRetries += 1
                    }
                    if pendingEmptyRetries >= 25 && (now - pendingEmptyStart) >= 0.40 {
                        lastChangeCount = count
                        pendingChangeCount = -1
                        pendingEmptyRetries = 0
                        pendingEmptyStart = 0
                    }
                }
            }
        } else {
            if !dragActive {
                pendingChangeCount = -1
                pendingEmptyRetries = 0
                pendingEmptyStart = 0
                if count != lastChangeCount {
                    lastChangeCount = count
                }
            }
            if dragActive && (!leftDown || forceEndIfUp) {
                dragActive = false
                dragWheelTriggeredForDrag = false
                pendingChangeCount = -1
                pendingEmptyRetries = 0
                pendingEmptyStart = 0
                lastEmittedX = -9999
                lastEmittedY = -9999
                let pos = currentCursorAndScreen()
                writeJSON([
                    "event": "drag_end",
                    "x": pos.x,
                    "y": pos.y
                ])
                dragFiles = []
                if count != lastChangeCount {
                    lastChangeCount = count
                }
            }
        }
    }

    private func tearDownMonitors() {
        dragKeyHeld = false
        dragKeyConsumed = false
        rawToggleKeyConsumed = false
        faceToggleKeyConsumed = false
        spaceToggleKeyConsumed = false
        settingsShortcutConsumed = false
        mouseTriggerHeld = false
        unregisterCarbonHotKey()
        activeTapMode = .none
        if let source = runLoopSource {
            CFRunLoopRemoveSource(CFRunLoopGetMain(), source, .commonModes)
            runLoopSource = nil
        }
        if let tap = eventTap {
            CGEvent.tapEnable(tap: tap, enable: false)
            CFMachPortInvalidate(tap)
            eventTap = nil
        }
        if let fm = fallbackMonitor {
            NSEvent.removeMonitor(fm)
            fallbackMonitor = nil
        }
    }

    private func ensureCarbonHotKeyHandlerInstalled() {
        guard carbonHotKeyHandlerRef == nil else { return }
        var eventType = EventTypeSpec(
            eventClass: OSType(kEventClassKeyboard),
            eventKind: UInt32(kEventHotKeyPressed)
        )
        let callback: EventHandlerUPP = { _, eventRef, _ -> OSStatus in
            guard let eventRef = eventRef else { return OSStatus(eventNotHandledErr) }
            var hotKeyID = EventHotKeyID()
            let status = GetEventParameter(
                eventRef,
                EventParamName(kEventParamDirectObject),
                EventParamType(typeEventHotKeyID),
                nil,
                MemoryLayout<EventHotKeyID>.size,
                nil,
                &hotKeyID
            )
            guard status == noErr,
                  hotKeyID.signature == MonitorController.carbonHotKeySignature,
                  hotKeyID.id == MonitorController.carbonHotKeyID else {
                return OSStatus(eventNotHandledErr)
            }
            MonitorController.shared.handleCarbonHotKeyPressed()
            return noErr
        }
        InstallEventHandler(
            GetEventDispatcherTarget(),
            callback,
            1,
            &eventType,
            nil,
            &carbonHotKeyHandlerRef
        )
    }

    private func unregisterCarbonHotKey() {
        if let hotKeyRef = carbonHotKeyRef {
            UnregisterEventHotKey(hotKeyRef)
            carbonHotKeyRef = nil
        }
    }

    private func syncCarbonHotKey() {
        unregisterCarbonHotKey()
        guard eventTap == nil, activeTapMode == .none, let idle = idleShortcut else {
            return
        }
        ensureCarbonHotKeyHandlerInstalled()
        var newRef: EventHotKeyRef?
        let hotKeyID = EventHotKeyID(
            signature: Self.carbonHotKeySignature,
            id: Self.carbonHotKeyID
        )
        let status = RegisterEventHotKey(
            UInt32(idle.keyCode),
            idle.carbonModifiers,
            hotKeyID,
            GetEventDispatcherTarget(),
            0,
            &newRef
        )
        if status == noErr, let ref = newRef {
            carbonHotKeyRef = ref
        }
    }

    private func handleCarbonHotKeyPressed() {
        guard !dragActive else { return }
        feedback.warmUp()
        let pos = currentCursorAndScreen()
        writeJSON([
            "event": "trigger_idle_wheel",
            "x": pos.x,
            "y": pos.y,
            "screen": pos.screen.toDict()
        ])
    }

    private func refreshAccessibilityMonitor(emitChange: Bool) {
        let nowAxTrusted = AXIsProcessTrusted()
        let nowListenTrusted = CGPreflightListenEventAccess()
        if nowAxTrusted == currentAxTrusted && nowListenTrusted == currentListenTrusted {
            return
        }
        let gainedAx = !currentAxTrusted && nowAxTrusted
        let gainedListen = !currentListenTrusted && nowListenTrusted
        setupInputMonitors(axTrusted: nowAxTrusted, listenTrusted: nowListenTrusted)
        if gainedAx || gainedListen {
            ActivePointerDetector.shared.refreshClient()
            LogitechHapticDriver.shared.reopenManager()
        }
        if emitChange {
            writeJSON([
                "event": "ax_status",
                "axTrusted": currentAxTrusted,
                "listenTrusted": currentListenTrusted,
                "tapMode": activeTapMode.rawValue
            ])
        }
    }

    private func attachEventTap(options: CGEventTapOptions, mask: Int, callback: CGEventTapCallBack) -> Bool {
        guard let tap = CGEvent.tapCreate(
            tap: .cgSessionEventTap,
            place: .headInsertEventTap,
            options: options,
            eventsOfInterest: CGEventMask(mask),
            callback: callback,
            userInfo: nil
        ) else {
            return false
        }
        self.eventTap = tap
        let source = CFMachPortCreateRunLoopSource(kCFAllocatorDefault, tap, 0)
        self.runLoopSource = source
        CFRunLoopAddSource(CFRunLoopGetMain(), source, .commonModes)
        CGEvent.tapEnable(tap: tap, enable: true)
        return true
    }

    private func setupInputMonitors(axTrusted: Bool, listenTrusted: Bool) {
        tearDownMonitors()
        monitorsInitialized = true
        currentListenTrusted = listenTrusted

        let mask = (1 << CGEventType.keyDown.rawValue)
            | (1 << CGEventType.keyUp.rawValue)
            | (1 << CGEventType.flagsChanged.rawValue)
            | (1 << CGEventType.otherMouseDown.rawValue)
            | (1 << CGEventType.otherMouseUp.rawValue)

        let callback: CGEventTapCallBack = { _, type, event, _ in
            return MonitorController.shared.handleCGEvent(type: type, event: event)
        }

        if axTrusted {
            if attachEventTap(options: .defaultTap, mask: mask, callback: callback) {
                activeTapMode = .defaultTap
                currentAxTrusted = true
                return
            }
        }

        currentAxTrusted = false
        if listenTrusted && attachEventTap(options: .listenOnly, mask: mask, callback: callback) {
            activeTapMode = .listenOnly
            return
        }

        activeTapMode = .none
        // Zero-permission fallback monitor (observes mouse/modifier events when CGEventTap is unavailable)
        self.fallbackMonitor = NSEvent.addGlobalMonitorForEvents(matching: [.keyDown, .keyUp, .flagsChanged, .otherMouseDown, .otherMouseUp]) { [weak self] event in
            self?.handleFallbackNSEvent(event)
        }
        syncCarbonHotKey()
    }

    private func handleCGEvent(type: CGEventType, event: CGEvent) -> Unmanaged<CGEvent>? {
        if type == .tapDisabledByTimeout || type == .tapDisabledByUserInput {
            if let tap = eventTap {
                CGEvent.tapEnable(tap: tap, enable: true)
            }
            return Unmanaged.passUnretained(event)
        }

        if type == .otherMouseDown || type == .otherMouseUp {
            let btn = event.getIntegerValueField(.mouseEventButtonNumber)
            if let targetBtn = mouseTriggerButton, btn == targetBtn {
                let pos = currentCursorAndScreen()
                if type == .otherMouseDown {
                    mouseTriggerHeld = true
                    feedback.warmUp()
                    writeJSON([
                        "event": "trigger_mouse_wheel",
                        "button": Int(btn),
                        "pressed": true,
                        "files": dragActive ? dragFiles : [],
                        "dragging": dragActive,
                        "x": pos.x,
                        "y": pos.y,
                        "screen": pos.screen.toDict()
                    ])
                } else {
                    mouseTriggerHeld = false
                    writeJSON([
                        "event": "mouse_release",
                        "button": Int(btn),
                        "x": pos.x,
                        "y": pos.y
                    ])
                }
                return nil // Consume configured trigger mouse button
            }
            return Unmanaged.passUnretained(event)
        }

        let keyCode = UInt16(event.getIntegerValueField(.keyboardEventKeycode))
        let flags = event.flags
        let isRepeat = event.getIntegerValueField(.keyboardEventAutorepeat) != 0

        if type == .keyDown && keyCode == 53 {
            writeJSON(["event": "escape"])
            if wheelOpen {
                return nil
            }
        }

        if (dragModifierOnly != nil || dragKeyCode != 15) && keyCode == 15 {
            let disallowedMods: CGEventFlags = [.maskCommand, .maskAlternate, .maskControl]
            if type == .keyDown {
                if !isRepeat {
                    if wheelOpen && flags.intersection(disallowedMods).isEmpty {
                        rawToggleKeyConsumed = true
                        writeJSON(["event": "toggle_raw"])
                        return nil
                    }
                    rawToggleKeyConsumed = false
                } else if rawToggleKeyConsumed {
                    return nil
                }
            } else if type == .keyUp && rawToggleKeyConsumed {
                rawToggleKeyConsumed = false
                return nil
            }
        }

        // T (keyCode 17) switches the wheel between its Formats and Tools faces, like R toggles Raw.
        if (dragModifierOnly != nil || dragKeyCode != 17) && keyCode == 17 {
            let disallowedMods: CGEventFlags = [.maskCommand, .maskAlternate, .maskControl]
            if type == .keyDown {
                if !isRepeat {
                    if wheelOpen && flags.intersection(disallowedMods).isEmpty {
                        faceToggleKeyConsumed = true
                        writeJSON(["event": "toggle_face"])
                        return nil
                    }
                    faceToggleKeyConsumed = false
                } else if faceToggleKeyConsumed {
                    return nil
                }
            } else if type == .keyUp && faceToggleKeyConsumed {
                faceToggleKeyConsumed = false
                return nil
            }
        }

        // Cmd+, (keyCode 43) opens FLIP Settings when the wheel is open.
        if keyCode == 43 {
            if type == .keyDown {
                if !isRepeat {
                    if wheelOpen && flags.contains(.maskCommand) && flags.intersection([.maskAlternate, .maskControl, .maskShift]).isEmpty {
                        settingsShortcutConsumed = true
                        writeJSON(["event": "open_settings"])
                        return nil
                    }
                    settingsShortcutConsumed = false
                } else if settingsShortcutConsumed {
                    return nil
                }
            } else if type == .keyUp && settingsShortcutConsumed {
                settingsShortcutConsumed = false
                return nil
            }
        }

        // Space (keyCode 49) toggles QuickLook preview during drag mode (wheel is unfocused).
        if (dragModifierOnly != nil || dragKeyCode != 49) && keyCode == 49 {
            let disallowedMods: CGEventFlags = [.maskCommand, .maskAlternate, .maskControl, .maskShift]
            if type == .keyDown {
                if !isRepeat {
                    if dragActive && wheelOpen && flags.intersection(disallowedMods).isEmpty {
                        spaceToggleKeyConsumed = true
                        writeJSON(["event": "wheel_space"])
                        return nil
                    }
                    spaceToggleKeyConsumed = false
                } else if spaceToggleKeyConsumed {
                    return nil
                }
            } else if type == .keyUp && spaceToggleKeyConsumed {
                spaceToggleKeyConsumed = false
                return nil
            }
        }

        // 1) Idle Global Shortcut
        if type == .keyDown && !isRepeat && !dragActive, let idle = idleShortcut, idle.matches(keyCode: keyCode, flags: flags) {
            feedback.warmUp()
            let pos = currentCursorAndScreen()
            writeJSON([
                "event": "trigger_idle_wheel",
                "x": pos.x,
                "y": pos.y,
                "screen": pos.screen.toDict()
            ])
            return nil
        }

        // 2) Drag Trigger Key (consumed ONLY when dragActive or already consumed for this press)
        if dragModifierOnly == nil && keyCode == dragKeyCode {
            let disallowedMods: CGEventFlags = [.maskCommand, .maskAlternate, .maskControl, .maskShift]
            let hasDisallowedMod = !flags.intersection(disallowedMods).isEmpty
            if type == .keyDown {
                if !isRepeat {
                    if hasDisallowedMod {
                        dragKeyHeld = false
                        dragKeyConsumed = false
                        return Unmanaged.passUnretained(event)
                    }
                    dragKeyHeld = true
                    if !dragActive {
                        checkDragState(forceReadPasteboard: true)
                    }
                    if dragActive {
                        dragKeyConsumed = true
                        triggerDragWheelIfNeeded(files: dragFiles)
                        return nil
                    }
                    dragKeyConsumed = false
                } else if dragKeyConsumed {
                    return nil
                }
            } else if type == .keyUp {
                dragKeyHeld = false
                if dragKeyConsumed {
                    dragKeyConsumed = false
                    return nil
                }
            }
        } else if type == .flagsChanged, let mod = dragModifierOnly {
            let pressed = modifierActive(mod, flags: flags)
            if pressed && !dragKeyHeld {
                dragKeyHeld = true
                if !dragActive {
                    checkDragState(forceReadPasteboard: true)
                }
                if dragActive {
                    triggerDragWheelIfNeeded(files: dragFiles)
                }
            } else if !pressed {
                dragKeyHeld = false
                dragKeyConsumed = false
            }
        }

        return Unmanaged.passUnretained(event)
    }

    private func handleFallbackNSEvent(_ event: NSEvent) {
        if event.type == .otherMouseDown || event.type == .otherMouseUp {
            let btn = Int64(event.buttonNumber)
            if let targetBtn = mouseTriggerButton, btn == targetBtn {
                let pos = currentCursorAndScreen()
                if event.type == .otherMouseDown {
                    mouseTriggerHeld = true
                    feedback.warmUp()
                    writeJSON([
                        "event": "trigger_mouse_wheel",
                        "button": Int(btn),
                        "pressed": true,
                        "files": dragActive ? dragFiles : [],
                        "dragging": dragActive,
                        "x": pos.x,
                        "y": pos.y,
                        "screen": pos.screen.toDict()
                    ])
                } else {
                    mouseTriggerHeld = false
                    writeJSON([
                        "event": "mouse_release",
                        "button": Int(btn),
                        "x": pos.x,
                        "y": pos.y
                    ])
                }
            }
            return
        }

        if event.type == .keyDown && event.keyCode == 53 {
            writeJSON(["event": "escape"])
            return
        }

        if event.type == .keyDown && !event.isARepeat && event.keyCode == 15 && wheelOpen && (dragModifierOnly != nil || dragKeyCode != 15) {
            let modFlags = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
            if modFlags.intersection([.command, .option, .control]).isEmpty {
                writeJSON(["event": "toggle_raw"])
                return
            }
        }

        if event.type == .keyDown && !event.isARepeat && event.keyCode == 17 && wheelOpen && (dragModifierOnly != nil || dragKeyCode != 17) {
            let modFlags = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
            if modFlags.intersection([.command, .option, .control]).isEmpty {
                writeJSON(["event": "toggle_face"])
                return
            }
        }

        if event.type == .keyDown && !event.isARepeat && event.keyCode == 43 && wheelOpen {
            let modFlags = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
            if modFlags.contains(.command) && modFlags.intersection([.option, .control, .shift]).isEmpty {
                writeJSON(["event": "open_settings"])
                return
            }
        }

        if event.type == .keyDown && !event.isARepeat && event.keyCode == 49 && dragActive && wheelOpen && (dragModifierOnly != nil || dragKeyCode != 49) {
            let modFlags = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
            if modFlags.intersection([.command, .option, .control, .shift]).isEmpty {
                writeJSON(["event": "wheel_space"])
                return
            }
        }

        if carbonHotKeyRef == nil, event.type == .keyDown && !event.isARepeat && !dragActive, let idle = idleShortcut, idle.matches(event: event) {
            feedback.warmUp()
            let pos = currentCursorAndScreen()
            writeJSON([
                "event": "trigger_idle_wheel",
                "x": pos.x,
                "y": pos.y,
                "screen": pos.screen.toDict()
            ])
            return
        }

        if dragModifierOnly == nil && event.keyCode == dragKeyCode {
            let modFlags = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
            let hasDisallowedMod = !modFlags.intersection([.command, .option, .control, .shift]).isEmpty
            if event.type == .keyDown && !event.isARepeat {
                if hasDisallowedMod {
                    dragKeyHeld = false
                    return
                }
                dragKeyHeld = true
                if !dragActive {
                    checkDragState(forceReadPasteboard: true)
                }
                if dragActive {
                    triggerDragWheelIfNeeded(files: dragFiles)
                }
            } else if event.type == .keyUp {
                dragKeyHeld = false
                dragKeyConsumed = false
            }
        } else if event.type == .flagsChanged, let mod = dragModifierOnly {
            let pressed = modifierActive(mod, nsFlags: event.modifierFlags)
            if pressed && !dragKeyHeld {
                dragKeyHeld = true
                if !dragActive {
                    checkDragState(forceReadPasteboard: true)
                }
                if dragActive {
                    triggerDragWheelIfNeeded(files: dragFiles)
                }
            } else if !pressed {
                dragKeyHeld = false
                dragKeyConsumed = false
            }
        }
    }

    private func modifierActive(_ mod: String, flags: CGEventFlags) -> Bool {
        switch mod {
        case "option", "alt": return flags.contains(.maskAlternate)
        case "control", "ctrl": return flags.contains(.maskControl)
        case "shift": return flags.contains(.maskShift)
        case "command", "cmd": return flags.contains(.maskCommand)
        case "fn": return flags.contains(.maskSecondaryFn)
        default: return false
        }
    }

    private func modifierActive(_ mod: String, nsFlags: NSEvent.ModifierFlags) -> Bool {
        switch mod {
        case "option", "alt": return nsFlags.contains(.option)
        case "control", "ctrl": return nsFlags.contains(.control)
        case "shift": return nsFlags.contains(.shift)
        case "command", "cmd": return nsFlags.contains(.command)
        case "fn": return nsFlags.contains(.function)
        default: return false
        }
    }
}
