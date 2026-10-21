import Foundation

/// Keypress-to-pixels tracker. Times are CACurrentMediaTime seconds in, milliseconds out.
public struct Latency {
    private var pending: [(seq: Int, at: Double)] = []
    private var samples: [Double] = []
    private var next = 0 // ring position once full
    public private(set) var total = 0 // samples ever taken
    public init() {}

    public var count: Int { samples.count }

    public mutating func start(seq: Int, at: Double) {
        pending.append((seq, at))
        if pending.count > 4096 { pending.removeFirst(2048) } // ponytail: bounded if frames never echo
    }

    /// Closes every started seq up to and including `seq`.
    public mutating func shown(seq: Int, at: Double) {
        while let p = pending.first, p.seq <= seq {
            pending.removeFirst()
            let ms = (at - p.at) * 1000
            if samples.count < 1000 { samples.append(ms) } else { samples[next] = ms; next = (next + 1) % 1000 }
            total += 1
        }
    }

    private func percentile(_ q: Double) -> Double {
        guard !samples.isEmpty else { return 0 }
        let s = samples.sorted()
        return s[min(s.count - 1, max(0, Int((q * Double(s.count)).rounded(.up)) - 1))]
    }

    public var p50: Double { percentile(0.5) }
    public var p99: Double { percentile(0.99) }

    public var summary: String { String(format: "latency p50 %.1f ms p99 %.1f ms (n=%d)", p50, p99, count) }
}
