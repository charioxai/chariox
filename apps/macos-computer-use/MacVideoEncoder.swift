import AppKit
import VideoToolbox

// Shared WebCodecs AVC packets use Annex B with in-band SPS/PPS. Each frame is
// independent, so abandoned credits cannot leave a dependent decoder chain.
func annexB(_ avcc: Data, parameterSets: [Data]) throws -> Data {
    var result = Data(), offset = 0
    for set in parameterSets { result.append(contentsOf: [0, 0, 0, 1]); result.append(set) }
    while offset < avcc.count {
        guard avcc.count - offset >= 4 else { throw tagged(.native, "codec-nal-length") }
        let count = avcc[offset..<(offset + 4)].reduce(0) { ($0 << 8) | Int($1) }
        offset += 4
        guard count > 0, count <= avcc.count - offset else { throw tagged(.native, "codec-nal-budget") }
        result.append(contentsOf: [0, 0, 0, 1]); result.append(avcc[offset..<(offset + count)])
        offset += count
    }
    guard !result.isEmpty, result.count <= 480 * 1024 else { throw tagged(.native, "codec-packet-budget") }
    return result
}

// Callback state owns at most one bounded packet, never a raw capture buffer.
private final class VideoResult: @unchecked Sendable {
    private let lock = NSLock()
    private var output: Data?
    private let completed = DispatchSemaphore(value: 0)
    func accept(_ status: OSStatus, _ flags: VTEncodeInfoFlags, _ sample: CMSampleBuffer?) {
        lock.lock(); defer { lock.unlock(); completed.signal() }
        guard status == noErr, !flags.contains(.frameDropped), let sample,
              CMSampleBufferDataIsReady(sample), let format = CMSampleBufferGetFormatDescription(sample),
              let block = CMSampleBufferGetDataBuffer(sample) else { return }
        let attachments = CMSampleBufferGetSampleAttachmentsArray(sample, createIfNecessary: false) as? [[String: Any]]
        guard attachments?.first?[kCMSampleAttachmentKey_NotSync as String] as? Bool != true else { return }
        var sets: [Data] = [], lengthSize: Int32 = 0
        for index in 0..<2 {
            var pointer: UnsafePointer<UInt8>?, size = 0, count = 0
            guard CMVideoFormatDescriptionGetH264ParameterSetAtIndex(format, parameterSetIndex: index,
                parameterSetPointerOut: &pointer, parameterSetSizeOut: &size, parameterSetCountOut: &count,
                nalUnitHeaderLengthOut: &lengthSize) == noErr, let pointer, size > 0, size <= 4096, count == 2, lengthSize == 4 else { return }
            sets.append(Data(bytes: pointer, count: size))
        }
        // Do not advertise a profile different from the shared negotiated codec.
        guard sets[0].count >= 4, sets[0][1] == 0x42, sets[0][2] == 0, sets[0][3] == 0x33 else { return }
        let length = CMBlockBufferGetDataLength(block)
        guard length > 0, length <= 480 * 1024 else { return }
        var bytes = Data(count: length)
        let copied = bytes.withUnsafeMutableBytes {
            CMBlockBufferCopyDataBytes(block, atOffset: 0, dataLength: length, destination: $0.baseAddress!)
        }
        guard copied == noErr else { return }
        output = try? annexB(bytes, parameterSets: sets)
    }
    func take() -> Data? {
        guard completed.wait(timeout: .now() + 0.2) == .success else { return nil }
        lock.lock(); defer { lock.unlock() }; return output }
}

protocol ProtectedVideoEncoding: AnyObject {
    func encode(_ frame: ProtectedFrame, sequence: UInt64) throws -> Data
}

final class MacVideoEncoder: ProtectedVideoEncoding {
    private var session: VTCompressionSession?
    private let width: Int, height: Int
    let backend: String
    init?(width: Int, height: Int, bitrate: Int, software: Bool = false) {
        guard width > 0, height > 0, width <= 4096, height <= 4096,
              width % 2 == 0, height % 2 == 0, width * height <= 4 * 1024 * 1024,
              (128_000...32_000_000).contains(bitrate) else { return nil }
        self.width = width; self.height = height
        var created: VTCompressionSession?
        let spec = [kVTVideoEncoderSpecification_EnableHardwareAcceleratedVideoEncoder: !software] as CFDictionary
        guard VTCompressionSessionCreate(allocator: nil, width: Int32(width), height: Int32(height), codecType: kCMVideoCodecType_H264,
            encoderSpecification: spec, imageBufferAttributes: nil, compressedDataAllocator: nil,
            outputCallback: nil, refcon: nil, compressionSessionOut: &created) == noErr, let created else { return nil }
        let properties: [CFString: Any] = [kVTCompressionPropertyKey_RealTime: true,
            kVTCompressionPropertyKey_AllowFrameReordering: false,
            kVTCompressionPropertyKey_ProfileLevel: kVTProfileLevel_H264_Baseline_5_1,
            kVTCompressionPropertyKey_AverageBitRate: bitrate,
            kVTCompressionPropertyKey_DataRateLimits: [bitrate / 8, 1],
            kVTCompressionPropertyKey_ExpectedFrameRate: 30,
            kVTCompressionPropertyKey_MaxKeyFrameInterval: 1,
            kVTCompressionPropertyKey_MaxFrameDelayCount: 0]
        guard VTSessionSetProperties(created, propertyDictionary: properties as CFDictionary) == noErr,
              VTCompressionSessionPrepareToEncodeFrames(created) == noErr else {
            VTCompressionSessionInvalidate(created); return nil
        }
        var actual: Unmanaged<CFTypeRef>?
        guard VTSessionCopyProperty(created, key: kVTCompressionPropertyKey_UsingHardwareAcceleratedVideoEncoder,
            allocator: nil, valueOut: &actual) == noErr, let hardware = actual?.takeRetainedValue() as? Bool,
              !software || !hardware else { VTCompressionSessionInvalidate(created); return nil }
        backend = hardware ? "videotoolbox-hardware" : "videotoolbox-software"
        session = created
    }
    deinit { if let session { VTCompressionSessionInvalidate(session) } }
    func encode(_ frame: ProtectedFrame, sequence: UInt64) throws -> Data {
        guard let session, frame.stamp.surface.width == width, frame.stamp.surface.height == height,
              frame.pixels.count == width * height * 4, sequence <= UInt64(Int64.max) else { throw tagged(.native, "codec-frame-binding") }
        // Convert only immutable protected sRGB RGBA. No SCK buffer enters VT.
        var buffer: CVPixelBuffer?
        guard CVPixelBufferCreate(nil, width, height, kCVPixelFormatType_32BGRA,
            [kCVPixelBufferIOSurfacePropertiesKey: [:]] as CFDictionary, &buffer) == kCVReturnSuccess,
              let buffer, CVPixelBufferLockBaseAddress(buffer, []) == kCVReturnSuccess else { throw tagged(.native, "codec-buffer") }
        let stride = CVPixelBufferGetBytesPerRow(buffer)
        let destination = CVPixelBufferGetBaseAddress(buffer)!.assumingMemoryBound(to: UInt8.self)
        for y in 0..<height { for x in 0..<width {
            let source = (y * width + x) * 4, target = y * stride + x * 4
            destination[target] = frame.pixels[source + 2]; destination[target + 1] = frame.pixels[source + 1]
            destination[target + 2] = frame.pixels[source]; destination[target + 3] = frame.pixels[source + 3]
        } }
        CVPixelBufferUnlockBaseAddress(buffer, [])
        CVBufferSetAttachment(buffer, kCVImageBufferColorPrimariesKey, kCVImageBufferColorPrimaries_ITU_R_709_2, .shouldPropagate)
        CVBufferSetAttachment(buffer, kCVImageBufferTransferFunctionKey, kCVImageBufferTransferFunction_sRGB, .shouldPropagate)
        CVBufferSetAttachment(buffer, kCVImageBufferYCbCrMatrixKey, kCVImageBufferYCbCrMatrix_ITU_R_709_2, .shouldPropagate)
        let result = VideoResult()
        let status = VTCompressionSessionEncodeFrame(session, imageBuffer: buffer,
            presentationTimeStamp: CMTime(value: Int64(sequence), timescale: 30), duration: CMTime(value: 1, timescale: 30),
            frameProperties: [kVTEncodeFrameOptionKey_ForceKeyFrame: true] as CFDictionary,
            infoFlagsOut: nil, outputHandler: { result.accept($0, $1, $2) })
        guard status == noErr, let bytes = result.take() else { throw tagged(.native, "codec-encode") }
        return bytes
    }
}
