// Disposable AppKit input oracle. Only UI events mutate its test state.
import AppKit
import CoreGraphics
import Foundation

final class EventView: NSView {
    var held = false
    var clicks = 0
    var doubles = 0
    var rights = 0
    var drags = 0
    var scrolls = 0
    var mouseUps = 0
    override var acceptsFirstResponder: Bool { true }
    override func draw(_ dirtyRect: NSRect) {
        NSColor.systemTeal.setFill(); bounds.fill()
        let text = "Pointer pad: click, double click, right click, drag, scroll"
        (text as NSString).draw(at: NSPoint(x:12,y:bounds.height/2), withAttributes:[.foregroundColor:NSColor.white])
    }
    override func mouseDown(with event: NSEvent) { held = true; clicks += 1; if event.clickCount == 2 { doubles += 1 }; window?.makeFirstResponder(self) }
    override func mouseUp(with event: NSEvent) { held = false; mouseUps += 1 }
    override func rightMouseDown(with event: NSEvent) { rights += 1 }
    override func mouseDragged(with event: NSEvent) { drags += 1 }
    override func scrollWheel(with event: NSEvent) { scrolls += 1 }
}
final class Delegate: NSObject, NSApplicationDelegate, NSTextFieldDelegate {
    let output: String
    var windows: [NSWindow] = []
    var fields: [NSTextField] = []
    let pad = EventView(frame:NSRect(x:20,y:60,width:500,height:170))
    var panel: NSPanel?
    var dialogField: NSTextField?
    var accepted = ""
    var keys: [[String:Any]] = []
    var timer: Timer?
    init(output: String) { self.output = output }
    func applicationDidFinishLaunching(_ notification: Notification) {
        for index in 0...1 {
            let window = NSWindow(contentRect:NSRect(x:500+index*600,y:450,width:540,height:340), styleMask:[.titled,.closable,.resizable], backing:.buffered,defer:false)
            window.title = index == 0 ? "Roder Cua Input A" : "Roder Cua Input B"
            window.isReleasedWhenClosed = false
            let field = NSTextField(frame:NSRect(x:20,y:275,width:500,height:30))
            field.placeholderString = index == 0 ? "Primary Unicode input" : "Sibling input"
            field.setAccessibilityLabel(index == 0 ? "Primary input" : "Sibling input")
            field.delegate = self
            window.contentView!.addSubview(field)
            fields.append(field); windows.append(window)
            if index == 0 {
                pad.setAccessibilityElement(true)
                pad.setAccessibilityRole(.group)
                pad.setAccessibilityLabel("Pointer pad")
                window.contentView!.addSubview(pad)
                let button = NSButton(title:"Open dialog",target:self,action:#selector(showDialog))
                button.frame = NSRect(x:20,y:20,width:150,height:30)
                window.contentView!.addSubview(button)
            }
            window.makeKeyAndOrderFront(nil)
        }
        NSApp.activate(ignoringOtherApps:true)
        windows[0].makeKeyAndOrderFront(nil)
        NSEvent.addLocalMonitorForEvents(matching:.keyDown) { event in
            self.keys.append(["characters":event.charactersIgnoringModifiers ?? "", "command":event.modifierFlags.contains(.command), "shift":event.modifierFlags.contains(.shift),"control":event.modifierFlags.contains(.control),"option":event.modifierFlags.contains(.option),"window":event.window?.title ?? ""])
            self.writeState()
            return event
        }
        timer = Timer.scheduledTimer(withTimeInterval:0.05,repeats:true) { _ in self.writeState() }
        writeState()
    }
    @objc func showDialog() {
        let panel = NSPanel(contentRect:NSRect(x:650,y:570,width:360,height:150),styleMask:[.titled,.closable],backing:.buffered,defer:false)
        panel.title = "Roder Cua Dialog"
        panel.isReleasedWhenClosed = false
        let field = NSTextField(frame:NSRect(x:20,y:85,width:320,height:30))
        field.setAccessibilityLabel("Dialog input")
        field.delegate = self
        panel.contentView!.addSubview(field)
        let button = NSButton(title:"Accept",target:self,action:#selector(acceptDialog))
        button.frame = NSRect(x:20,y:20,width:120,height:30)
        panel.contentView!.addSubview(button)
        self.panel = panel; self.dialogField = field
        panel.makeKeyAndOrderFront(nil)
        panel.makeFirstResponder(field)
        writeState()
    }
    @objc func acceptDialog() {
        accepted = dialogField?.stringValue ?? ""
        panel?.close(); panel = nil
        windows[0].makeKeyAndOrderFront(nil)
        writeState()
    }
    func controlTextDidChange(_ notification: Notification) { writeState() }
    func writeState() {
        let cursor = CGEvent(source:nil)?.location ?? .zero
        let state: [String:Any] = ["cursor_x":Double(cursor.x),"cursor_y":Double(cursor.y),"pid":ProcessInfo.processInfo.processIdentifier,"fields":fields.map(\.stringValue),"held":pad.held,"clicks":pad.clicks,"doubles":pad.doubles,"rights":pad.rights,"drags":pad.drags,"scrolls":pad.scrolls,"mouse_ups":pad.mouseUps,"keys":keys,"dialog_open":panel != nil,"dialog_value":dialogField?.stringValue ?? "","accepted":accepted,"key_window":NSApp.keyWindow?.title ?? "", "windows":windows.map { ["title":$0.title,"number":$0.windowNumber,"width":$0.frame.width,"height":$0.frame.height,"x":$0.frame.origin.x,"y":$0.frame.origin.y] }]
        if let data = try? JSONSerialization.data(withJSONObject:state,options:[.sortedKeys]) {
            try? data.write(to:URL(fileURLWithPath:output),options:.atomic)
        }
    }
}
let args = CommandLine.arguments
if args.count != 3 || args[1] != "--state" { fatalError("usage: InputFixture --state PATH") }
let app = NSApplication.shared
app.setActivationPolicy(.regular)
let delegate = Delegate(output:args[2])
app.delegate = delegate
app.run()
