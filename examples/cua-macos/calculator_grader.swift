// Independent, read-only Vision OCR of the final window PNG. No GUI input.
import Foundation
import Vision
import ImageIO

guard CommandLine.arguments.count == 2 else { fatalError("usage: calculator_grader PNG") }
let source = CGImageSourceCreateWithURL(URL(fileURLWithPath:CommandLine.arguments[1]) as CFURL,nil)!
let image = CGImageSourceCreateImageAtIndex(source,0,nil)!
let request = VNRecognizeTextRequest()
request.recognitionLevel = .accurate
request.usesLanguageCorrection = false
try VNImageRequestHandler(cgImage:image).perform([request])
let rows = (request.results ?? []).compactMap { observation -> [String:Any]? in
    guard let text = observation.topCandidates(1).first else { return nil }
    return ["text":text.string,"confidence":text.confidence,"x":Double(observation.boundingBox.minX),"y":Double(observation.boundingBox.minY)]
}
// Require the result in the display above the button grid, not a key label.
let passed = rows.contains { ($0["text"] as? String) == "42" && ($0["y"] as? Double ?? 0) > 0.65 }
let result: [String:Any] = ["passed":passed,"source":"independent Apple Vision OCR of final native Calculator PNG","recognized":rows]
let data = try JSONSerialization.data(withJSONObject:result,options:[.sortedKeys])
print(String(data:data,encoding:.utf8)!)
