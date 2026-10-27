import SwiftUI
import UIKit
import Darwin

@main
struct TetoPhoneSpikeApp: App {
    var body: some Scene { WindowGroup { SpikeView() } }
}

struct SpikeView: View {
    @Environment(\.scenePhase) private var scenePhase
    @State private var running = false
    @State private var status = "Import the pinned E2B GGUF into Documents. iPhone 15 Pro Max is the qualification device."
    var body: some View {
        VStack(spacing: 24) {
            Text(status)
            Button("Run five-minute feasibility spike", action: begin).disabled(running)
            Button("Run six-case cache probe", action: beginProbe).disabled(running)
            Button("Stop after current clause") { teto_phone_cancel() }.disabled(!running)
        }.padding()
        .task {
            if ProcessInfo.processInfo.arguments.contains("--cache-probe") {
                beginProbe()
            } else if ProcessInfo.processInfo.arguments.contains("--autorun") {
                begin()
            }
        }
        .onChange(of: scenePhase) { _, phase in
            if phase == .background && running { teto_phone_cancel() }
        }
    }

    private func begin() {
        guard !running else { return }
        running = true
        UIApplication.shared.isIdleTimerDisabled = true
        status = "Running in foreground. Audio qualification remains pending."
        let documents = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
        let stem = "spike-\(UUID().uuidString)"
        let report = documents.appendingPathComponent(stem + ".jsonl")
        let resources = documents.appendingPathComponent(stem + "-resources.jsonl")
        Task {
            let monitor = Task {
                while !Task.isCancelled {
                    appendSample(resources)
                    do { try await Task.sleep(nanoseconds: 5_000_000_000) } catch { break }
                }
            }
            let result = await Task.detached { runSpike(documents, report) }.value
            monitor.cancel()
            appendSample(resources)
            UIApplication.shared.isIdleTimerDisabled = false
            status = result == 0 ? "Complete: \(report.lastPathComponent). Export both JSONL files."
                : result == 2 ? "Interrupted. Preserve both reports; this run cannot qualify the duration."
                : "Spike failed. Preserve reports and device diagnostics."
            running = false
        }
    }

    private func beginProbe() {
        guard !running else { return }
        running = true
        UIApplication.shared.isIdleTimerDisabled = true
        status = "Running the six-case cache probe in the foreground."
        let documents = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0]
        let report = documents.appendingPathComponent("cache-\(UUID().uuidString).jsonl")
        Task {
            let result = await Task.detached { runCacheProbe(documents, report) }.value
            UIApplication.shared.isIdleTimerDisabled = false
            status = result == 0 ? "Cache probe passed: \(report.lastPathComponent)."
                : "Cache probe failed: \(report.lastPathComponent). Export the report."
            running = false
        }
    }

    private func runSpike(_ documents: URL, _ report: URL) -> Int32 {
        setHost()
        let assets = Array(documents.path.utf8)
        let destination = Array(report.path.utf8)
        return assets.withUnsafeBufferPointer { source in
            destination.withUnsafeBufferPointer { target in
                teto_phone_spike(source.baseAddress, source.count, target.baseAddress, target.count)
            }
        }
    }

    private func runCacheProbe(_ documents: URL, _ report: URL) -> Int32 {
        setHost()
        let assets = Array(documents.path.utf8)
        let destination = Array(report.path.utf8)
        return assets.withUnsafeBufferPointer { source in
            destination.withUnsafeBufferPointer { target in
                teto_phone_cache_probe(
                    source.baseAddress, source.count, target.baseAddress, target.count)
            }
        }
    }

    private func setHost() {
        let device = Array(machineName().utf8)
        let os = Array(UIDevice.current.systemVersion.utf8)
        device.withUnsafeBufferPointer { deviceBuffer in
            os.withUnsafeBufferPointer { osBuffer in
                _ = teto_phone_set_host(
                    deviceBuffer.baseAddress, deviceBuffer.count,
                    osBuffer.baseAddress, osBuffer.count)
            }
        }
    }

    private func machineName() -> String {
        var systemInfo = utsname()
        uname(&systemInfo)
        return withUnsafePointer(to: &systemInfo.machine) {
            $0.withMemoryRebound(to: CChar.self, capacity: 1) {
                String(cString: $0)
            }
        }
    }
}

private func memoryUsage() -> (footprint: UInt64, resident: UInt64)? {
    var info = task_vm_info_data_t()
    var count = mach_msg_type_number_t(MemoryLayout<task_vm_info_data_t>.size / MemoryLayout<integer_t>.size)
    let result = withUnsafeMutablePointer(to: &info) { pointer in
        pointer.withMemoryRebound(to: integer_t.self, capacity: Int(count)) {
            task_info(mach_task_self_, task_flavor_t(TASK_VM_INFO), $0, &count)
        }
    }
    return result == KERN_SUCCESS ? (info.phys_footprint, info.resident_size) : nil
}

private func appendSample(_ destination: URL) {
    var value: [String: Any] = ["uptime_seconds": ProcessInfo.processInfo.systemUptime,
        "thermal_state": ProcessInfo.processInfo.thermalState.rawValue,
        "os": ProcessInfo.processInfo.operatingSystemVersionString,
        "physical_memory_bytes": ProcessInfo.processInfo.physicalMemory,
        "device": UIDevice.current.model, "foreground": UIApplication.shared.applicationState == .active, "audio": "pending: real TTS required"]
    if let usage = memoryUsage() {
        value["physical_footprint_bytes"] = usage.footprint
        value["resident_bytes"] = usage.resident
    }
    guard var bytes = try? JSONSerialization.data(withJSONObject: value) else { return }
    bytes.append(10)
    if !FileManager.default.fileExists(atPath: destination.path) {
        FileManager.default.createFile(atPath: destination.path, contents: nil)
    }
    guard let file = try? FileHandle(forWritingTo: destination) else { return }
    defer { try? file.close() }
    do { try file.seekToEnd(); try file.write(contentsOf: bytes); try file.synchronize() } catch { }
}
