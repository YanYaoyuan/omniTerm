// Copyright 2026 omniTerm contributors. SPDX-License-Identifier: Apache-2.0
import AppKit

let size = NSSize(width: 1024, height: 1024)
let image = NSImage(size: size)
image.lockFocus()
let body = NSBezierPath(roundedRect: NSRect(x: 48, y: 48, width: 928, height: 928), xRadius: 210, yRadius: 210)
NSColor(red: 0.085, green: 0.115, blue: 0.16, alpha: 1).setFill()
body.fill()
let inner = NSBezierPath(roundedRect: NSRect(x: 86, y: 86, width: 852, height: 852), xRadius: 172, yRadius: 172)
NSColor(red: 0.22, green: 0.29, blue: 0.39, alpha: 1).setStroke()
inner.lineWidth = 7
inner.stroke()
let glyph = NSBezierPath()
glyph.move(to: NSPoint(x: 260, y: 678))
glyph.line(to: NSPoint(x: 455, y: 512))
glyph.line(to: NSPoint(x: 260, y: 346))
glyph.lineWidth = 61
glyph.lineCapStyle = .round
glyph.lineJoinStyle = .round
NSColor(red: 0.55, green: 0.71, blue: 1, alpha: 1).setStroke()
glyph.stroke()
let cursor = NSBezierPath()
cursor.move(to: NSPoint(x: 535, y: 346))
cursor.line(to: NSPoint(x: 750, y: 346))
cursor.lineWidth = 61
cursor.lineCapStyle = .round
cursor.stroke()
NSColor(red: 0.84, green: 0.9, blue: 1, alpha: 1).setFill()
NSBezierPath(ovalIn: NSRect(x: 724, y: 704, width: 66, height: 66)).fill()
image.unlockFocus()
let bitmap = NSBitmapImageRep(data: image.tiffRepresentation!)!
let png = bitmap.representation(using: .png, properties: [:])!
try png.write(to: URL(fileURLWithPath: CommandLine.arguments[1]))
