// Local PDFKit/Vision adapter. Reads one JSON request from stdin and returns JSON.
// File access and user dialogs are owned by the Rust application; inputs are bytes.
import Foundation
import AppKit
import PDFKit
import Vision
import CoreGraphics

enum MediaError: Error { case invalid(String) }
func invalid(_ text: String) throws -> Never { throw MediaError.invalid(text) }
func integer(_ object: [String: Any], _ key: String, _ fallback: Int) -> Int {
    (object[key] as? NSNumber)?.intValue ?? fallback
}
func number(_ object: [String: Any], _ key: String, _ fallback: Double) -> Double {
    (object[key] as? NSNumber)?.doubleValue ?? fallback
}
func decode(_ value: Any?) throws -> Data {
    guard let text = value as? String, let data = Data(base64Encoded: text), !data.isEmpty,
          data.count <= 64 * 1024 * 1024 else { try invalid("无效或过大的媒体数据") }
    return data
}
func document(_ value: Any?) throws -> PDFDocument {
    guard let doc = PDFDocument(data: try decode(value)), doc.pageCount > 0, doc.pageCount <= 500 else {
        try invalid("PDF 无法读取，或页数超出 500 页")
    }
    guard !doc.isLocked else { try invalid("加密 PDF 请先在预览中解锁并另存副本") }
    return doc
}
func selected(_ value: Any?, _ count: Int, _ max: Int) throws -> [Int] {
    let pages = (value as? [NSNumber])?.map { $0.intValue } ?? Array(0..<count)
    guard !pages.isEmpty, pages.count <= max, pages.allSatisfy({ $0 >= 0 && $0 < count }) else {
        try invalid("请选择有效的 PDF 页码并分批处理")
    }
    return pages
}
func displaySize(_ page: PDFPage) -> CGSize {
    let box = page.bounds(for: .cropBox)
    return abs(page.rotation % 180) == 90 ? CGSize(width: box.height, height: box.width) : box.size
}
func render(_ page: PDFPage, _ dpi: Double, maxWidth: Int? = nil) throws -> (Data, CGImage) {
    let size = displaySize(page)
    guard size.width.isFinite, size.height.isFinite, size.width > 0, size.height > 0 else {
        try invalid("PDF 页面尺寸无效")
    }
    let scale = maxWidth.map { Double($0) / size.width } ?? dpi / 72
    let width = max(1, Int((size.width * scale).rounded()))
    let height = max(1, Int((size.height * scale).rounded()))
    guard width <= 12000, height <= 12000, width * height <= 24_000_000,
          let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: width, pixelsHigh: height,
                                    bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true,
                                    isPlanar: false, colorSpaceName: .deviceRGB,
                                    bytesPerRow: 0, bitsPerPixel: 0),
          let graphics = NSGraphicsContext(bitmapImageRep: rep) else { try invalid("PDF 渲染尺寸过大") }
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = graphics
    NSColor.white.setFill()
    NSRect(x: 0, y: 0, width: width, height: height).fill()
    page.thumbnail(of: CGSize(width: width, height: height), for: .cropBox)
        .draw(in: NSRect(x: 0, y: 0, width: width, height: height))
    graphics.flushGraphics()
    NSGraphicsContext.restoreGraphicsState()
    guard let png = rep.representation(using: .png, properties: [:]), let cg = rep.cgImage else {
        try invalid("PDF 页面无法渲染")
    }
    return (png, cg)
}
func ocr(_ page: PDFPage) throws -> (String, Double) {
    let (_, image) = try render(page, 160, maxWidth: 1800)
    let request = VNRecognizeTextRequest()
    request.recognitionLevel = .accurate
    request.usesLanguageCorrection = true
    request.recognitionLanguages = ["zh-Hans", "en-US"]
    if #available(macOS 13, *) { request.automaticallyDetectsLanguage = true }
    try VNImageRequestHandler(cgImage: image, options: [:]).perform([request])
    let recognized = (request.results ?? []).compactMap { observation -> (String, Float)? in
        guard let candidate = observation.topCandidates(1).first else { return nil }
        return (candidate.string, candidate.confidence)
    }
    let confidence = recognized.isEmpty ? 0 : Double(recognized.map { $0.1 }.reduce(0,+)) / Double(recognized.count)
    return (recognized.map { $0.0 }.joined(separator: "\n"), confidence)
}
func pdfResult(_ doc: PDFDocument) throws -> [String: Any] {
    guard let bytes = doc.dataRepresentation(), bytes.count <= 128 * 1024 * 1024 else {
        try invalid("PDF 导出失败或文件超出 128 MB")
    }
    return ["data": "data:application/pdf;base64," + bytes.base64EncodedString(),
            "bytes": bytes.count, "mime": "application/pdf", "pageCount": doc.pageCount]
}
func color(_ hex: String) -> NSColor {
    let raw = hex.hasPrefix("#") ? String(hex.dropFirst()) : hex
    let rgb = UInt32(raw, radix: 16) ?? 0x24364B
    return NSColor(calibratedRed: Double((rgb >> 16) & 255)/255,
                   green: Double((rgb >> 8) & 255)/255, blue: Double(rgb & 255)/255, alpha: 1)
}
func annotationBounds(_ annotation: [String: Any], _ page: PDFPage) -> CGRect {
    let size = displaySize(page)
    let display = CGRect(x: number(annotation,"x",0.08) * size.width,
                         y: (1-number(annotation,"y",0.08)-number(annotation,"height",0.12)) * size.height,
                         width: number(annotation,"width",0.8) * size.width,
                         height: number(annotation,"height",0.12) * size.height)
    guard let ref = page.pageRef else { return display }
    let transform = ref.getDrawingTransform(.cropBox, rect: CGRect(origin: .zero, size: size),
                                           rotate: 0, preserveAspectRatio: false).inverted()
    return display.applying(transform)
}
func dispatch(_ req: [String: Any]) throws -> [String: Any] {
    let action = req["action"] as? String ?? ""
    if action == "pdfMerge" {
        guard let inputs = req["documents"] as? [String], inputs.count >= 2, inputs.count <= 20 else {
            try invalid("合并需要 2–20 个 PDF")
        }
        let merged = PDFDocument()
        for input in inputs {
            let source = try document(input)
            guard merged.pageCount + source.pageCount <= 500 else { try invalid("合并后超出 500 页") }
            for i in 0..<source.pageCount {
                guard let page = source.page(at: i)?.copy() as? PDFPage else { try invalid("PDF 页面复制失败") }
                merged.insert(page, at: merged.pageCount)
            }
        }
        return try pdfResult(merged)
    }
    if action == "imagesToPdf" {
        guard let inputs = req["images"] as? [String], !inputs.isEmpty, inputs.count <= 30 else {
            try invalid("请选择 1–30 张图片")
        }
        let buffer = NSMutableData()
        guard let consumer = CGDataConsumer(data: buffer as CFMutableData) else { try invalid("PDF 创建失败") }
        let mode = req["pageSize"] as? String ?? "a4"
        var box = CGRect(x:0,y:0,width:595.276,height:841.89)
        guard let context = CGContext(consumer: consumer, mediaBox: &box, nil) else { try invalid("PDF 创建失败") }
        for input in inputs {
            guard let nsimage = NSImage(data: try decode(input)),
                  let image = nsimage.cgImage(forProposedRect: nil, context: nil, hints: nil) else {
                try invalid("图片无法读取")
            }
            var page = mode == "original" ? CGRect(x:0,y:0,width:Double(image.width)*0.75,height:Double(image.height)*0.75) : box
            let mediaBoxData = Data(bytes:&page,count:MemoryLayout<CGRect>.size)
            context.beginPDFPage([kCGPDFContextMediaBox as String: mediaBoxData] as CFDictionary)
            context.setFillColor(CGColor(gray:1,alpha:1)); context.fill(page)
            let margin = mode == "original" ? 0.0 : 36.0
            let scale = min((page.width-2*margin)/Double(image.width),(page.height-2*margin)/Double(image.height))
            let width = Double(image.width)*scale, height=Double(image.height)*scale
            context.draw(image, in: CGRect(x:(page.width-width)/2,y:(page.height-height)/2,width:width,height:height))
            context.endPDFPage()
        }
        context.closePDF()
        guard let pdf = PDFDocument(data: buffer as Data) else { try invalid("图片转 PDF 失败") }
        return try pdfResult(pdf)
    }
    let source = try document(req["data"])
    if action == "pdfInspect" {
        let preview = Set((req["previewPages"] as? [NSNumber] ?? []).map { $0.intValue })
        var pages = [[String: Any]]()
        var characters = 0
        for i in 0..<source.pageCount {
            guard let page = source.page(at:i) else { try invalid("PDF 页面无法读取") }
            let size = displaySize(page), length = page.string?.count ?? 0
            characters += length
            var info: [String: Any] = ["index":i,"width":size.width,"height":size.height,
                                      "rotation":page.rotation,"textLength":length]
            if preview.contains(i) {
                let (png,_) = try render(page,72,maxWidth:360)
                info["preview"] = "data:image/png;base64," + png.base64EncodedString()
            }
            pages.append(info)
        }
        return ["pageCount":source.pageCount,"pages":pages,"textAvailable":characters>0,"textLength":characters]
    }
    if action == "pdfRender" {
        let indices = try selected(req["pages"],source.pageCount,10)
        let dpi = number(req,"dpi",120)
        var results = [[String: Any]]()
        var total = 0
        for index in indices {
            guard let page=source.page(at:index) else { try invalid("PDF 页面无法读取") }
            let (png,cg)=try render(page,dpi)
            total += png.count
            guard total<=64*1024*1024 else { try invalid("渲染结果超过 64 MB，请减少页数或分辨率") }
            results.append(["index":index,"data":"data:image/png;base64,"+png.base64EncodedString(),
                            "width":cg.width,"height":cg.height,"mime":"image/png"])
        }
        return ["images":results]
    }
    if action == "pdfToText" {
        let mode = req["ocr"] as? String ?? "auto"
        let indices=try selected(req["pages"],source.pageCount,500)
        var results = [[String: Any]](), allText = [String]()
        var usedOcr = false, ocrPages = 0
        for index in indices {
            guard let page=source.page(at:index) else { try invalid("PDF 页面无法读取") }
            let embedded=(page.string ?? "").trimmingCharacters(in:.whitespacesAndNewlines)
            let useOcr=mode=="always" || (mode=="auto" && embedded.isEmpty)
            var text=embedded, confidence=1.0
            if useOcr {
                ocrPages += 1
                guard ocrPages<=20 else { try invalid("单次 OCR 最多 20 页，请选择页码后分批转换") }
                (text,confidence)=try ocr(page); usedOcr=true
            }
            guard allText.reduce(0,{$0+$1.count})+text.count<=2_000_000 else { try invalid("PDF 文本超过 200 万字，请分批转换") }
            results.append(["index":index,"text":text,"source":useOcr ? "ocr" : "embedded","confidence":confidence])
            allText.append(text)
        }
        return ["text":allText.joined(separator:"\n\n"),"pages":results,"usedOcr":usedOcr]
    }
    if action == "pdfTransform" {
        let entries = req["pages"] as? [[String:Any]] ?? (0..<source.pageCount).map { ["index":$0,"rotation":0] }
        guard !entries.isEmpty, entries.count<=500 else { try invalid("需要保留至少一页，最多 500 页") }
        let result=PDFDocument()
        for entry in entries {
            let index=integer(entry,"index",-1), rotation=integer(entry,"rotation",0)
            guard index>=0,index<source.pageCount,rotation%90==0,
                  let page=source.page(at:index)?.copy() as? PDFPage else { try invalid("PDF 页码或旋转角度无效") }
            page.rotation=((page.rotation+rotation)%360+360)%360
            result.insert(page,at:result.pageCount)
        }
        for annotation in req["annotations"] as? [[String:Any]] ?? [] {
            let index=integer(annotation,"page",-1)
            guard index>=0,index<result.pageCount,let page=result.page(at:index),
                  let text=annotation["text"] as? String,!text.isEmpty else { try invalid("批注页码或文字无效") }
            let note=PDFAnnotation(bounds:annotationBounds(annotation,page),forType:.freeText,withProperties:nil)
            note.contents=text
            note.font=NSFont(name:"PingFang SC",size:number(annotation,"fontSize",14)) ?? NSFont.systemFont(ofSize:14)
            note.fontColor=color(annotation["color"] as? String ?? "#24364B")
            note.color=NSColor.clear
            note.alignment = .left
            let border=PDFBorder();border.lineWidth=0;note.border=border
            note.setValue(page.rotation,forAnnotationKey:PDFAnnotationKey(rawValue:"Rotate"))
            page.addAnnotation(note)
        }
        return try pdfResult(result)
    }
    try invalid("未知 PDF 操作")
}

do {
    let input=FileHandle.standardInput.readDataToEndOfFile()
    guard input.count<=256*1024*1024,let request=try JSONSerialization.jsonObject(with:input) as? [String:Any] else {
        try invalid("无效媒体请求")
    }
    let output=try dispatch(request)
    FileHandle.standardOutput.write(try JSONSerialization.data(withJSONObject:output,options:[.sortedKeys]))
} catch {
    let message: String
    if case MediaError.invalid(let text)=error { message=text } else { message="PDF 处理失败，请检查文件或减少处理页数" }
    let output=try! JSONSerialization.data(withJSONObject:["error":message],options:[.sortedKeys])
    FileHandle.standardOutput.write(output)
}
