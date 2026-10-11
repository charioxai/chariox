// Independent decoder consumes the public protected Annex B artifact. It shares
// no encoder conversion or packet builder and uses a software decoder request.
import AppKit
import VideoToolbox

@main struct CodecDecodeTests {
    static func main() throws {
        let bytes = [UInt8](try Data(contentsOf: URL(fileURLWithPath: CommandLine.arguments[1])))
        var units: [[UInt8]] = [], position = 0
        while position < bytes.count {
            precondition(position + 4 <= bytes.count && Array(bytes[position..<(position+4)]) == [0,0,0,1])
            let start = position + 4
            position = start
            while position + 4 <= bytes.count && Array(bytes[position..<(position+4)]) != [0,0,0,1] { position += 1 }
            if position + 4 > bytes.count { position = bytes.count }
            units.append(Array(bytes[start..<position]))
        }
        precondition(units.count >= 3 && units[0][0] & 31 == 7 && units[1][0] & 31 == 8)
        var format: CMFormatDescription?
        let status = units[0].withUnsafeBufferPointer { sps in units[1].withUnsafeBufferPointer { pps in
            let pointers = [sps.baseAddress!, pps.baseAddress!], sizes = [sps.count, pps.count]
            return CMVideoFormatDescriptionCreateFromH264ParameterSets(allocator: nil, parameterSetCount: 2,
                parameterSetPointers: pointers, parameterSetSizes: sizes, nalUnitHeaderLength: 4, formatDescriptionOut: &format)
        } }
        precondition(status == noErr)
        var payload = Data()
        for unit in units.dropFirst(2) {
            var length = UInt32(unit.count).bigEndian
            withUnsafeBytes(of: &length) { payload.append(contentsOf: $0) }; payload.append(contentsOf: unit)
        }
        var block: CMBlockBuffer?
        precondition(CMBlockBufferCreateWithMemoryBlock(allocator: nil, memoryBlock: nil, blockLength: payload.count,
            blockAllocator: nil, customBlockSource: nil, offsetToData: 0, dataLength: payload.count, flags: 0, blockBufferOut: &block) == noErr)
        precondition(payload.withUnsafeBytes { CMBlockBufferReplaceDataBytes(with: $0.baseAddress!, blockBuffer: block!, offsetIntoDestination: 0, dataLength: payload.count) } == noErr)
        var sample: CMSampleBuffer?, timing = CMSampleTimingInfo(duration: .invalid, presentationTimeStamp: .zero, decodeTimeStamp: .invalid)
        var size = payload.count
        precondition(CMSampleBufferCreateReady(allocator: nil, dataBuffer: block, formatDescription: format!, sampleCount: 1,
            sampleTimingEntryCount: 1, sampleTimingArray: &timing, sampleSizeEntryCount: 1, sampleSizeArray: &size, sampleBufferOut: &sample) == noErr)
        var decoder: VTDecompressionSession?
        precondition(VTDecompressionSessionCreate(allocator: nil, formatDescription: format!,
            decoderSpecification: [kVTVideoDecoderSpecification_EnableHardwareAcceleratedVideoDecoder: false] as CFDictionary,
            imageBufferAttributes: [kCVPixelBufferPixelFormatTypeKey: kCVPixelFormatType_32BGRA] as CFDictionary,
            outputCallback: nil, decompressionSessionOut: &decoder) == noErr)
        defer { VTDecompressionSessionInvalidate(decoder!) }
        var observed = false
        let decoded = VTDecompressionSessionDecodeFrame(decoder!, sampleBuffer: sample!, flags: [], infoFlagsOut: nil) { status, _, image, _, _ in
            precondition(status == noErr && image != nil)
            let image = image!
            precondition(CVPixelBufferGetWidth(image) == 128 && CVPixelBufferGetHeight(image) == 128)
            precondition(CVPixelBufferLockBaseAddress(image, .readOnly) == kCVReturnSuccess)
            defer { CVPixelBufferUnlockBaseAddress(image, .readOnly) }
            let pixels = CVPixelBufferGetBaseAddress(image)!.assumingMemoryBound(to: UInt8.self)
            let stride = CVPixelBufferGetBytesPerRow(image)
            // Interior of the already-masked public black raster stays black.
            for y in 16..<112 { for x in 16..<112 {
                let i = y * stride + x * 4
                precondition(pixels[i] < 8 && pixels[i+1] < 8 && pixels[i+2] < 8)
            } }
            observed = true
        }
        precondition(decoded == noErr && observed)
        print("PASS independent software H264 decode, native dimensions and protected black interior")
    }
}
