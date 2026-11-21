import Foundation
import AppKit
import CoreGraphics
import QuartzCore

final class AuraPassThroughWindow: NSWindow {
    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }
}

final class AuraEdgeView: NSView {
    private let containerLayer = CALayer()
    private let glowContainerLayer = CALayer()
    private let glowCornerMask = CAShapeLayer()
    private let topGradient = CAGradientLayer()
    private let bottomGradient = CAGradientLayer()
    private let leftGradient = CAGradientLayer()
    private let rightGradient = CAGradientLayer()
    private let topMiterMask = CAShapeLayer()
    private let bottomMiterMask = CAShapeLayer()
    private let leftMiterMask = CAShapeLayer()
    private let rightMiterMask = CAShapeLayer()
    private let borderLayer = CAShapeLayer()

    private let glowDepth: CGFloat = 32.0
    private let borderInset: CGFloat = 3.0
    private let borderWidth: CGFloat = 3.0
    private let cornerRadius: CGFloat = 12.0

    private var currentColor: NSColor = NSColor(srgbRed: 1.0, green: 0.48, blue: 0.10, alpha: 1.0)

    override var isOpaque: Bool { false }

    override init(frame frameRect: NSRect) {
        super.init(frame: frameRect)
        setupLayers()
    }

    required init?(coder: NSCoder) {
        super.init(coder: coder)
        setupLayers()
    }

    private func setupLayers() {
        wantsLayer = true
        if layer == nil {
            layer = CALayer()
        }
        layer?.backgroundColor = NSColor.clear.cgColor

        containerLayer.backgroundColor = NSColor.clear.cgColor
        containerLayer.opacity = 0.0

        glowContainerLayer.backgroundColor = NSColor.clear.cgColor
        glowCornerMask.fillColor = NSColor.white.cgColor
        glowContainerLayer.mask = glowCornerMask
        containerLayer.addSublayer(glowContainerLayer)

        // Non-flipped macOS CoreAnimation coordinates: (0,0) is bottom-left, (1,1) is top-right.
        topGradient.startPoint = CGPoint(x: 0.5, y: 1.0)
        topGradient.endPoint = CGPoint(x: 0.5, y: 0.0)

        bottomGradient.startPoint = CGPoint(x: 0.5, y: 0.0)
        bottomGradient.endPoint = CGPoint(x: 0.5, y: 1.0)

        leftGradient.startPoint = CGPoint(x: 0.0, y: 0.5)
        leftGradient.endPoint = CGPoint(x: 1.0, y: 0.5)

        rightGradient.startPoint = CGPoint(x: 1.0, y: 0.5)
        rightGradient.endPoint = CGPoint(x: 0.0, y: 0.5)

        let locations: [NSNumber] = [0.0, 0.35, 1.0]
        let edgePairs: [(CAGradientLayer, CAShapeLayer)] = [
            (topGradient, topMiterMask),
            (bottomGradient, bottomMiterMask),
            (leftGradient, leftMiterMask),
            (rightGradient, rightMiterMask),
        ]
        for (gradient, miterMask) in edgePairs {
            gradient.locations = locations
            miterMask.fillColor = NSColor.white.cgColor
            miterMask.allowsEdgeAntialiasing = false
            gradient.mask = miterMask
            glowContainerLayer.addSublayer(gradient)
        }

        borderLayer.fillColor = nil
        borderLayer.lineWidth = borderWidth
        containerLayer.addSublayer(borderLayer)

        layer?.addSublayer(containerLayer)
        applyColor(currentColor)
        updateTransparencyMode()
    }

    private func makeQuadPath(_ p0: CGPoint, _ p1: CGPoint, _ p2: CGPoint, _ p3: CGPoint) -> CGPath {
        let path = CGMutablePath()
        path.move(to: p0)
        path.addLine(to: p1)
        path.addLine(to: p2)
        path.addLine(to: p3)
        path.closeSubpath()
        return path
    }

    override func layout() {
        super.layout()
        CATransaction.begin()
        CATransaction.setDisableActions(true)

        containerLayer.frame = bounds
        glowContainerLayer.frame = bounds
        glowCornerMask.frame = bounds
        let glowClipRect = bounds.insetBy(dx: borderInset * 0.5, dy: borderInset * 0.5)
        let clampedGlowRadius = min(cornerRadius, min(glowClipRect.width, glowClipRect.height) * 0.5)
        glowCornerMask.path = CGPath(
            roundedRect: glowClipRect,
            cornerWidth: max(0, clampedGlowRadius),
            cornerHeight: max(0, clampedGlowRadius),
            transform: nil
        )

        let w = bounds.width
        let h = bounds.height
        let d = min(glowDepth, min(w, h) * 0.5)

        topGradient.frame = CGRect(x: 0, y: h - d, width: w, height: d)
        topMiterMask.frame = topGradient.bounds
        topMiterMask.path = makeQuadPath(
            CGPoint(x: 0, y: d),
            CGPoint(x: d, y: 0),
            CGPoint(x: w - d, y: 0),
            CGPoint(x: w, y: d)
        )

        bottomGradient.frame = CGRect(x: 0, y: 0, width: w, height: d)
        bottomMiterMask.frame = bottomGradient.bounds
        bottomMiterMask.path = makeQuadPath(
            CGPoint(x: 0, y: 0),
            CGPoint(x: d, y: d),
            CGPoint(x: w - d, y: d),
            CGPoint(x: w, y: 0)
        )

        leftGradient.frame = CGRect(x: 0, y: 0, width: d, height: h)
        leftMiterMask.frame = leftGradient.bounds
        leftMiterMask.path = makeQuadPath(
            CGPoint(x: 0, y: 0),
            CGPoint(x: d, y: d),
            CGPoint(x: d, y: h - d),
            CGPoint(x: 0, y: h)
        )

        rightGradient.frame = CGRect(x: w - d, y: 0, width: d, height: h)
        rightMiterMask.frame = rightGradient.bounds
        rightMiterMask.path = makeQuadPath(
            CGPoint(x: d, y: 0),
            CGPoint(x: 0, y: d),
            CGPoint(x: 0, y: h - d),
            CGPoint(x: d, y: h)
        )

        borderLayer.frame = bounds
        let strokeRect = bounds.insetBy(dx: borderInset, dy: borderInset)
        let clampedBorderRadius = min(cornerRadius, min(strokeRect.width, strokeRect.height) * 0.5)
        borderLayer.path = CGPath(
            roundedRect: strokeRect,
            cornerWidth: max(0, clampedBorderRadius),
            cornerHeight: max(0, clampedBorderRadius),
            transform: nil
        )

        updateTransparencyMode()
        CATransaction.commit()
    }

    func updateTransparencyMode() {
        let reduceTransparency = NSWorkspace.shared.accessibilityDisplayShouldReduceTransparency
        glowContainerLayer.isHidden = reduceTransparency
        for gradient in [topGradient, bottomGradient, leftGradient, rightGradient] {
            gradient.isHidden = reduceTransparency
        }
    }

    func setColor(_ color: NSColor) {
        currentColor = color
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        applyColor(color)
        updateTransparencyMode()
        CATransaction.commit()
    }

    private func applyColor(_ color: NSColor) {
        let cgColors: [CGColor] = [
            color.withAlphaComponent(0.72).cgColor,
            color.withAlphaComponent(0.24).cgColor,
            color.withAlphaComponent(0.0).cgColor
        ]
        for gradient in [topGradient, bottomGradient, leftGradient, rightGradient] {
            gradient.colors = cgColors
        }
        borderLayer.strokeColor = color.withAlphaComponent(0.95).cgColor
    }

    func startBreathing(reduceMotion: Bool) {
        containerLayer.removeAnimation(forKey: "aura.breath")
        containerLayer.removeAnimation(forKey: "aura.fade")

        if reduceMotion {
            CATransaction.begin()
            CATransaction.setDisableActions(true)
            containerLayer.opacity = 0.82
            CATransaction.commit()
            return
        }

        CATransaction.begin()
        CATransaction.setDisableActions(true)
        containerLayer.opacity = 0.52
        CATransaction.commit()

        let breath = CABasicAnimation(keyPath: "opacity")
        breath.fromValue = 0.52
        breath.toValue = 0.95
        breath.duration = 1.35
        breath.autoreverses = true
        breath.repeatCount = .infinity
        breath.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
        breath.isRemovedOnCompletion = false
        containerLayer.add(breath, forKey: "aura.breath")
    }

    func showSolid(opacity: Float) {
        containerLayer.removeAnimation(forKey: "aura.breath")
        containerLayer.removeAnimation(forKey: "aura.fade")
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        containerLayer.opacity = opacity
        CATransaction.commit()
    }

    func fadeOut(duration: TimeInterval, immediate: Bool) {
        let currentOpacity = containerLayer.presentation()?.opacity ?? containerLayer.opacity
        containerLayer.removeAnimation(forKey: "aura.breath")
        containerLayer.removeAnimation(forKey: "aura.fade")

        CATransaction.begin()
        CATransaction.setDisableActions(true)
        containerLayer.opacity = 0.0
        CATransaction.commit()

        if immediate || duration <= 0.0 {
            return
        }

        let fade = CABasicAnimation(keyPath: "opacity")
        fade.fromValue = currentOpacity
        fade.toValue = 0.0
        fade.duration = duration
        fade.timingFunction = CAMediaTimingFunction(name: .easeOut)
        fade.fillMode = .forwards
        fade.isRemovedOnCompletion = false
        containerLayer.add(fade, forKey: "aura.fade")
    }
}

final class AuraOverlayController {
    static let shared = AuraOverlayController()

    private enum AuraState {
        case off
        case active
        case success
    }

    private static let activeOrange = NSColor(srgbRed: 1.0, green: 0.48, blue: 0.10, alpha: 1.0)
    private static let successGreen = NSColor(srgbRed: 0.18, green: 0.84, blue: 0.45, alpha: 1.0)

    var isEnabled: Bool = true {
        didSet {
            if !isEnabled {
                hide(force: true)
            }
        }
    }

    private var state: AuraState = .off
    private var window: AuraPassThroughWindow?
    private var edgeView: AuraEdgeView?
    private var pendingWorkItems: [DispatchWorkItem] = []
    private var transitionGeneration: UInt64 = 0
    private var accessibilityObserver: NSObjectProtocol?

    private init() {
        accessibilityObserver = NSWorkspace.shared.notificationCenter.addObserver(
            forName: NSWorkspace.accessibilityDisplayOptionsDidChangeNotification,
            object: nil,
            queue: .main
        ) { [weak self] _ in
            self?.handleAccessibilityOptionsChanged()
        }
    }

    deinit {
        if let observer = accessibilityObserver {
            NSWorkspace.shared.notificationCenter.removeObserver(observer)
        }
    }

    private func handleAccessibilityOptionsChanged() {
        edgeView?.updateTransparencyMode()
        if state == .active {
            edgeView?.startBreathing(reduceMotion: NSWorkspace.shared.accessibilityDisplayShouldReduceMotion)
        }
    }

    private func runOnMain(_ block: @escaping () -> Void) {
        if Thread.isMainThread {
            block()
        } else {
            DispatchQueue.main.async(execute: block)
        }
    }

    private func cancelPendingTimers() {
        for item in pendingWorkItems {
            item.cancel()
        }
        pendingWorkItems.removeAll()
        transitionGeneration &+= 1
    }

    private func targetScreen() -> NSScreen? {
        let mouseLocation = NSEvent.mouseLocation
        if let hit = NSScreen.screens.first(where: { NSMouseInRect(mouseLocation, $0.frame, false) }) {
            return hit
        }
        return NSScreen.main ?? NSScreen.screens.first
    }

    private func prepareWindowOnActiveScreen() -> (AuraPassThroughWindow, AuraEdgeView)? {
        guard let screen = targetScreen() else { return nil }
        let frame = screen.frame

        if let win = window, let view = edgeView {
            if win.frame != frame {
                win.setFrame(frame, display: true)
            }
            view.frame = NSRect(origin: .zero, size: frame.size)
            view.needsLayout = true
            view.layoutSubtreeIfNeeded()
            return (win, view)
        }

        let win = AuraPassThroughWindow(
            contentRect: frame,
            styleMask: [.borderless],
            backing: .buffered,
            defer: false
        )
        win.ignoresMouseEvents = true
        win.collectionBehavior = [.canJoinAllSpaces, .fullScreenAuxiliary, .ignoresCycle, .stationary]
        win.isOpaque = false
        win.backgroundColor = .clear
        win.hasShadow = false
        win.level = NSWindow.Level(rawValue: Int(CGWindowLevelForKey(.statusWindow)))
        win.animationBehavior = .none
        win.isReleasedWhenClosed = false

        let view = AuraEdgeView(frame: NSRect(origin: .zero, size: frame.size))
        view.autoresizingMask = [.width, .height]
        win.contentView = view
        view.needsLayout = true
        view.layoutSubtreeIfNeeded()

        self.window = win
        self.edgeView = view
        return (win, view)
    }

    func showActive() {
        runOnMain { [weak self] in
            guard let self = self, self.isEnabled else { return }
            self.cancelPendingTimers()
            guard let (win, view) = self.prepareWindowOnActiveScreen() else { return }

            self.state = .active
            view.setColor(Self.activeOrange)
            view.startBreathing(reduceMotion: NSWorkspace.shared.accessibilityDisplayShouldReduceMotion)
            win.orderFrontRegardless()
        }
    }

    func showSuccess() {
        runOnMain { [weak self] in
            guard let self = self, self.isEnabled else { return }
            self.cancelPendingTimers()
            guard let (win, view) = self.prepareWindowOnActiveScreen() else { return }

            self.state = .success
            view.setColor(Self.successGreen)
            view.showSolid(opacity: 0.98)
            win.orderFrontRegardless()

            let gen = self.transitionGeneration
            let reduceMotion = NSWorkspace.shared.accessibilityDisplayShouldReduceMotion

            let fadeWork = DispatchWorkItem { [weak self] in
                guard let self = self, self.transitionGeneration == gen, self.state == .success else { return }
                if !reduceMotion {
                    self.edgeView?.fadeOut(duration: 0.480, immediate: false)
                }
            }
            let orderOutWork = DispatchWorkItem { [weak self] in
                guard let self = self, self.transitionGeneration == gen, self.state == .success else { return }
                self.edgeView?.fadeOut(duration: 0.0, immediate: true)
                self.state = .off
                self.window?.orderOut(nil)
            }

            self.pendingWorkItems.append(fadeWork)
            self.pendingWorkItems.append(orderOutWork)
            DispatchQueue.main.asyncAfter(deadline: .now() + .milliseconds(220), execute: fadeWork)
            DispatchQueue.main.asyncAfter(deadline: .now() + .milliseconds(700), execute: orderOutWork)
        }
    }

    func hide(force: Bool = false) {
        runOnMain { [weak self] in
            guard let self = self else { return }
            if !force && self.state == .success {
                return
            }

            self.cancelPendingTimers()
            if self.state == .off && !(self.window?.isVisible ?? false) {
                return
            }

            self.state = .off
            let reduceMotion = NSWorkspace.shared.accessibilityDisplayShouldReduceMotion
            if !self.isEnabled || reduceMotion {
                self.edgeView?.fadeOut(duration: 0.0, immediate: true)
                self.window?.orderOut(nil)
                return
            }

            let gen = self.transitionGeneration
            self.edgeView?.fadeOut(duration: 0.160, immediate: false)

            let orderOutWork = DispatchWorkItem { [weak self] in
                guard let self = self, self.transitionGeneration == gen, self.state == .off else { return }
                self.window?.orderOut(nil)
            }
            self.pendingWorkItems.append(orderOutWork)
            DispatchQueue.main.asyncAfter(deadline: .now() + .milliseconds(165), execute: orderOutWork)
        }
    }
}
