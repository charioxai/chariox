import AppKit

func protectedRaster(_ image: CGImage, surface: NativeSurface, masks: [CGRect], wholeWindowMask: Bool)
    throws -> (image: CGImage, pixels: [UInt8]) {
    guard surface.width > 0, surface.height > 0, surface.width * surface.height <= 4 * 1024 * 1024,
          image.width == surface.width, image.height == surface.height,
          masks.allSatisfy({ !$0.isNull && $0.width > 0 && $0.height > 0 &&
              [$0.minX, $0.minY, $0.width, $0.height].allSatisfy { $0.isFinite } }) else {
        throw tagged(.secure, "capture-mask-geometry-unknown")
    }
    var pixels = [UInt8](repeating: 0, count: surface.width * surface.height * 4)
    let protectedImage: CGImage = try pixels.withUnsafeMutableBytes { data in
            guard let context = CGContext(data: data.baseAddress, width: surface.width, height: surface.height,
                bitsPerComponent: 8, bytesPerRow: surface.width * 4, space: CGColorSpace(name: CGColorSpace.sRGB)!,
                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue | CGBitmapInfo.byteOrder32Big.rawValue) else {
                throw tagged(.native, "capture-bitmap")
            }
            context.draw(image, in: CGRect(x: 0, y: 0, width: surface.width, height: surface.height))
            context.setFillColor(CGColor(gray: 0, alpha: 1))
            if wholeWindowMask { context.fill(CGRect(x: 0, y: 0, width: surface.width, height: surface.height)) }
            else {
                for mask in masks {
                    let rect = mask.intersection(surface.bounds)
                    if rect.isNull || rect.isEmpty { continue }
                    context.fill(CGRect(x: (rect.minX - surface.bounds.minX) * surface.scale,
                        y: (surface.bounds.maxY - rect.maxY) * surface.scale,
                        width: rect.width * surface.scale, height: rect.height * surface.scale).insetBy(dx: -2, dy: -2).integral)
                }
            }
            guard let result = context.makeImage() else { throw tagged(.native, "capture-protected-image") }
            return result
        }
    return (protectedImage, pixels)
}
