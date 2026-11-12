@file:Suppress("DEPRECATION")

package dev.azurice.guiuse

import android.app.KeyguardManager
import android.content.Context
import android.os.SystemClock
import android.view.accessibility.AccessibilityNodeInfo
import dev.azurice.guiuse.protocol.ProtocolEvents
import dev.azurice.guiuse.protocol.ScreenStateData
import java.time.Instant
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject

class DeviceRpcException(val code: Int, override val message: String) : Exception(message)

object AgentRuntime {
    val json = Json { ignoreUnknownKeys = false; encodeDefaults = true }
    @Volatile var accessibility: DeviceService? = null
    @Volatile var notifications: NotificationService? = null
    @Volatile var connectionStatus: String = "Disconnected"
    @Volatile var activityName: String = ""
    @Volatile var lastUiEvent: Long = SystemClock.uptimeMillis()
    private val lock = Any()
    private var sink: ((JsonObject) -> Boolean)? = null
    private var acceptedEvents = emptySet<String>()
    @Volatile var sequence: Int = 0
        private set

    fun connected(events: Set<String>, send: (JsonObject) -> Boolean) = synchronized(lock) {
        sequence = 0
        acceptedEvents = events
        sink = send
    }

    fun disconnected() = synchronized(lock) { sink = null; acceptedEvents = emptySet() }

    fun emit(name: String, build: (Int, String) -> JsonObject) = synchronized(lock) {
        val send = sink ?: return@synchronized
        if (name !in acceptedEvents) return@synchronized
        if (sequence == Int.MAX_VALUE) { sink = null; return@synchronized }
        sequence += 1
        // Failed sends retain a sequence gap; the host must reconcile with snapshots.
        send(build(sequence, Instant.now().toString()))
    }

    fun screenState(context: Context) {
        val power = context.getSystemService(android.os.PowerManager::class.java)
        val keyguard = context.getSystemService(KeyguardManager::class.java)
        emit("screen.state") { seq, ts ->
            ProtocolEvents.screenState(seq, ts, ScreenStateData(power.isInteractive, power.isInteractive, keyguard.isKeyguardLocked), json)
        }
    }
}

class SafetyPolicy(private val context: Context) {
    fun requirePackage(packageName: String) {
        if (packageName !in AgentSettings(context).allowedPackages) {
            throw DeviceRpcException(2005, "Application is not allowlisted")
        }
        val lowered = packageName.lowercase()
        if (listOf("bank", "wallet", "alipay", "gpay", "payment").any { lowered.contains(it) }) {
            throw DeviceRpcException(2003, "Sensitive application is blocked")
        }
    }

    fun requireSafeRoot(root: AccessibilityNodeInfo) {
        if (context.getSystemService(KeyguardManager::class.java).isKeyguardLocked) {
            throw DeviceRpcException(2003, "Device is locked")
        }
        requirePackage(root.packageName?.toString() ?: "")
        val stack = java.util.ArrayDeque<AccessibilityNodeInfo>()
        stack.add(AccessibilityNodeInfo.obtain(root))
        var inspected = 0
        try {
            while (stack.isNotEmpty()) {
                val node = stack.removeLast()
                try {
                    inspected += 1
                    if (inspected > 10000) throw DeviceRpcException(2003, "Screen exceeds safety inspection limit")
                    val text = "${node.text ?: ""} ${node.contentDescription ?: ""}".lowercase()
                    if (node.isPassword || listOf("验证码", "付款", "支付", "one-time password", "verification code", "card number").any(text::contains)) {
                        throw DeviceRpcException(2003, "Sensitive screen is blocked")
                    }
                    for (index in 0 until node.childCount) node.getChild(index)?.let(stack::add)
                } finally { node.recycle() }
            }
        } finally { stack.forEach { it.recycle() } }
    }
}
