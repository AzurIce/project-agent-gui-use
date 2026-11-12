@file:Suppress("DEPRECATION")

package dev.azurice.guiuse

import android.accessibilityservice.AccessibilityService
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityNodeInfo
import android.view.accessibility.AccessibilityWindowInfo
import dev.azurice.guiuse.protocol.ProtocolEvents
import dev.azurice.guiuse.protocol.UiContentChangedData
import dev.azurice.guiuse.protocol.UiIdleData
import dev.azurice.guiuse.protocol.UiTextChangedData
import dev.azurice.guiuse.protocol.UiWindowChangedData

class DeviceService : AccessibilityService() {
    private val handler = Handler(Looper.getMainLooper())
    private var content: Runnable? = null
    private var idle: Runnable? = null
    var invalidateSnapshot: (() -> Unit)? = null

    override fun onServiceConnected() { AgentRuntime.accessibility = this }

    fun applicationRoot(): AccessibilityNodeInfo? {
        val current = windows
        return try {
            // The IME may become the accessibility-active window while the input
            // focus still belongs to the application. Never pick a background app
            // merely because its package happens to be allowlisted.
            current.firstOrNull { it.type == AccessibilityWindowInfo.TYPE_APPLICATION && it.isFocused }?.root
                ?: rootInActiveWindow
        } finally { current.forEach { it.recycle() } }
    }

    override fun onAccessibilityEvent(event: AccessibilityEvent?) {
        event ?: return
        val packageName = event.packageName?.toString() ?: return
        if (packageName !in AgentSettings(this).allowedPackages) return
        AgentRuntime.lastUiEvent = SystemClock.uptimeMillis()
        if (event.eventType == AccessibilityEvent.TYPE_WINDOW_STATE_CHANGED) {
            AgentRuntime.activityName = event.className?.toString() ?: ""
            // Window-state events can arrive after a snapshot of the same window.
            // Actions validate the current window and refresh the cached node instead
            // of invalidating IDs merely because a delayed event was delivered.
            AgentRuntime.emit("ui.window_changed") { seq, ts ->
                ProtocolEvents.uiWindowChanged(seq, ts, UiWindowChangedData(packageName, AgentRuntime.activityName), AgentRuntime.json)
            }
        }
        if (event.eventType == AccessibilityEvent.TYPE_VIEW_TEXT_CHANGED && !event.isPassword) {
            // Inspect before forwarding text, not just before performing actions.
            val root = applicationRoot()
            if (root != null) {
                val safe = runCatching { SafetyPolicy(this).requireSafeRoot(root) }.isSuccess
                root.recycle()
                val source = event.source
                val hint = try { source?.viewIdResourceName ?: source?.className?.toString() ?: event.className?.toString() ?: "" }
                    finally { source?.recycle() }
                if (safe) AgentRuntime.emit("ui.text_changed") { seq, ts ->
                    ProtocolEvents.uiTextChanged(seq, ts, UiTextChangedData(hint, event.text.joinToString("").take(16384)), AgentRuntime.json)
                }
            }
        }
        content?.let(handler::removeCallbacks)
        content = Runnable {
            AgentRuntime.emit("ui.content_changed") { seq, ts ->
                ProtocolEvents.uiContentChanged(seq, ts, UiContentChangedData(packageName, AgentRuntime.activityName, listOf("content")), AgentRuntime.json)
            }
        }.also { handler.postDelayed(it, 300) }
        idle?.let(handler::removeCallbacks)
        idle = Runnable {
            val since = AgentRuntime.sequence
            AgentRuntime.emit("ui.idle") { seq, ts -> ProtocolEvents.uiIdle(seq, ts, UiIdleData(since), AgentRuntime.json) }
        }.also { handler.postDelayed(it, 350) }
    }

    override fun onInterrupt() = Unit
    override fun onDestroy() {
        handler.removeCallbacksAndMessages(null)
        invalidateSnapshot?.invoke()
        if (AgentRuntime.accessibility === this) AgentRuntime.accessibility = null
        super.onDestroy()
    }
}
