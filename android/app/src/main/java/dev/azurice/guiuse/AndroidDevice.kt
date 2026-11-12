@file:Suppress("DEPRECATION")

package dev.azurice.guiuse

import android.accessibilityservice.AccessibilityService
import android.accessibilityservice.GestureDescription
import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.pm.ApplicationInfo
import android.graphics.Bitmap
import android.graphics.Path
import android.graphics.Rect
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.view.Display
import android.view.accessibility.AccessibilityNodeInfo
import dev.azurice.guiuse.protocol.*
import java.io.ByteArrayOutputStream
import kotlin.coroutines.AbstractCoroutineContextElement
import kotlin.coroutines.CoroutineContext
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.delay
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withTimeout
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException

class RequestFrame : AbstractCoroutineContextElement(Key) {
    companion object Key : CoroutineContext.Key<RequestFrame>
    var binary: ByteArray? = null
}

class AndroidDevice(private val context: Context) : Device {
    private val policy = SafetyPolicy(context)
    private val cached = mutableMapOf<String, AccessibilityNodeInfo>()
    private var generation = 0
    private var snapshotWindow = -1

    fun clearNodes() {
        cached.values.forEach { it.recycle() }
        cached.clear()
    }

    private fun service(): DeviceService = AgentRuntime.accessibility
        ?.also { it.invalidateSnapshot = ::clearNodes }
        ?: throw DeviceRpcException(2005, "Accessibility permission is not enabled")

    private fun root(): AccessibilityNodeInfo {
        val root = service().applicationRoot() ?: throw DeviceRpcException(2005, "No accessible active window")
        try { policy.requireSafeRoot(root) }
        catch (error: Exception) { root.recycle(); throw error }
        return root
    }

    override suspend fun appList(includeSystem: Boolean?): AppListResult {
        val pm = context.packageManager
        val launcher = Intent(Intent.ACTION_MAIN).addCategory(Intent.CATEGORY_LAUNCHER)
        val apps = pm.queryIntentActivities(launcher, 0).map { it.activityInfo.applicationInfo }
            .distinctBy { it.packageName }
            .filter { includeSystem == true || it.flags and (ApplicationInfo.FLAG_SYSTEM or ApplicationInfo.FLAG_UPDATED_SYSTEM_APP) == 0 }
            .map { AppInfo(it.packageName, pm.getApplicationLabel(it).toString(), true) }
            .sortedBy { it.package_name }
        return AppListResult(apps)
    }

    override suspend fun appLaunch(packageName: String, activity: String?, forceStop: Boolean?): AppLaunchResult {
        policy.requirePackage(packageName)
        if (forceStop == true) throw DeviceRpcException(2002, "Force-stop requires privileges unavailable to an Android application")
        val intent = if (activity == null) context.packageManager.getLaunchIntentForPackage(packageName)
        else Intent().setComponent(ComponentName(packageName, if (activity.startsWith('.')) packageName + activity else activity))
        if (intent == null || intent.resolveActivity(context.packageManager) == null) throw DeviceRpcException(2006, "No launchable activity")
        AgentRuntime.lastUiEvent = SystemClock.uptimeMillis()
        try { context.startActivity(intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) }
        catch (_: android.content.ActivityNotFoundException) { throw DeviceRpcException(2006, "Activity is unavailable") }
        return AppLaunchResult(true)
    }

    override suspend fun uiSnapshot(filter: String?, maxNodes: Int?): UiSnapshotResult {
        val mode = filter ?: "actionable"
        if (mode !in setOf("actionable", "all")) throw DeviceRpcException(-32602, "Unknown snapshot filter")
        val limit = maxNodes ?: 800
        if (limit !in 1..10000) throw DeviceRpcException(-32602, "max_nodes must be between 1 and 10000")
        val root = root()
        clearNodes()
        generation += 1
        snapshotWindow = root.windowId
        val packageName = root.packageName?.toString() ?: ""
        data class Pending(val node: AccessibilityNodeInfo, val parent: String?, val depth: Int)
        val stack = java.util.ArrayDeque<Pending>()
        stack.add(Pending(root, null, 0))
        val result = mutableListOf<Node>()
        var inspected = 0
        var textBudget = 500000
        var truncated = false
        try {
            while (stack.isNotEmpty()) {
                val (node, parent, depth) = stack.removeLast()
                if (node.isPassword) {
                    node.recycle(); clearNodes()
                    throw DeviceRpcException(2003, "Password field appeared during snapshot")
                }
                inspected += 1
                if (inspected > 10000 || depth > 128 || result.size >= limit) {
                    node.recycle(); truncated = true; break
                }
                val emitted = mode == "all" || (node.isVisibleToUser &&
                    (node.isClickable || node.isEditable || node.isScrollable || node.isCheckable || node.isFocusable))
                val id = if (emitted) "s$generation-n${result.size}" else parent
                if (emitted) {
                    val text = node.text?.toString()?.take(minOf(16384, textBudget))
                    textBudget -= text?.length ?: 0
                    val description = node.contentDescription?.toString()?.take(minOf(16384, textBudget))
                    textBudget -= description?.length ?: 0
                    if ((node.text?.length ?: 0) > (text?.length ?: 0) || (node.contentDescription?.length ?: 0) > (description?.length ?: 0)) truncated = true
                    val bounds = Rect().also(node::getBoundsInScreen)
                    result.add(Node(id!!, parent, node.className?.toString() ?: "",
                        text, description, node.viewIdResourceName,
                        Bounds(bounds.left, bounds.top, bounds.right, bounds.bottom),
                        NodeFlags(node.isClickable, node.isEditable, node.isScrollable, node.isCheckable, node.isEnabled, node.isFocused, node.isSelected, node.isChecked),
                        supportedActions(node)))
                    cached[id] = node
                }
                for (index in node.childCount - 1 downTo 0) node.getChild(index)?.let { stack.add(Pending(it, id, depth + 1)) }
                if (!emitted) node.recycle()
            }
        } finally { stack.forEach { it.node.recycle() } }
        return UiSnapshotResult(AgentRuntime.sequence, packageName, AgentRuntime.activityName, truncated, result)
    }

    private fun supportedActions(node: AccessibilityNodeInfo): List<String> = node.actionList.mapNotNull { action ->
        actions.entries.firstOrNull { it.value == action.id }?.key
    }.let { if (node.isEditable && "set_text" in it) it + "clear_text" else it }

    override suspend fun uiAct(node: String, action: String, text: String?): UiActResult {
        val root = root()
        val window = root.windowId
        root.recycle()
        val target = cached[node] ?: throw DeviceRpcException(2001, "Node is not part of the current snapshot")
        if (window != snapshotWindow || !target.refresh()) throw DeviceRpcException(2001, "Node is stale")
        if (target.isPassword) throw DeviceRpcException(2003, "Password field is blocked")
        val actionId = if (action == "clear_text") AccessibilityNodeInfo.ACTION_SET_TEXT else actions[action]
        if (actionId == null || action !in supportedActions(target)) throw DeviceRpcException(2002, "Node does not support this action")
        val arguments = Bundle()
        if (actionId == AccessibilityNodeInfo.ACTION_SET_TEXT) {
            val value = if (action == "clear_text") "" else text ?: throw DeviceRpcException(-32602, "set_text requires text")
            if (value.length > 32768) throw DeviceRpcException(-32602, "Text is too long")
            arguments.putCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE, value)
        }
        AgentRuntime.lastUiEvent = SystemClock.uptimeMillis()
        return UiActResult(target.performAction(actionId, arguments))
    }

    override suspend fun uiGesture(strokes: List<GestureStroke>, timeoutMs: Int?): UiGestureResult {
        root().recycle()
        if (strokes.size !in 1..10) throw DeviceRpcException(-32602, "Gesture requires 1 to 10 strokes")
        val metrics = context.resources.displayMetrics
        val builder = GestureDescription.Builder()
        var end = 0L
        for (stroke in strokes) {
            val start = (stroke.start_ms ?: 0).toLong()
            val duration = (stroke.duration_ms ?: 300).toLong()
            if (stroke.points.size !in 1..1000 || start < 0 || duration <= 0 || start + duration > 60000) throw DeviceRpcException(-32602, "Invalid gesture timing or point count")
            if (stroke.points.any { it.x !in 0 until metrics.widthPixels || it.y !in 0 until metrics.heightPixels }) throw DeviceRpcException(-32602, "Gesture coordinates are outside the screen")
            val path = Path()
            stroke.points.forEachIndexed { index, point -> if (index == 0) path.moveTo(point.x.toFloat(), point.y.toFloat()) else path.lineTo(point.x.toFloat(), point.y.toFloat()) }
            builder.addStroke(GestureDescription.StrokeDescription(path, start, duration))
            end = maxOf(end, start + duration)
        }
        val timeout = timeoutMs?.toLong() ?: (end + 1000)
        if (timeout !in 1..65000) throw DeviceRpcException(-32602, "Invalid gesture timeout")
        return withTimeout(timeout) {
            suspendCancellableCoroutine { continuation ->
                val accepted = service().dispatchGesture(builder.build(), object : AccessibilityService.GestureResultCallback() {
                    override fun onCompleted(gestureDescription: GestureDescription?) {
                        AgentRuntime.lastUiEvent = SystemClock.uptimeMillis()
                        if (continuation.isActive) continuation.resume(UiGestureResult(true))
                    }
                    override fun onCancelled(gestureDescription: GestureDescription?) { if (continuation.isActive) continuation.resume(UiGestureResult(false)) }
                }, Handler(Looper.getMainLooper()))
                if (!accepted && continuation.isActive) continuation.resumeWithException(DeviceRpcException(2002, "Gesture was rejected"))
            }
        }
    }

    override suspend fun uiWait(until: String, value: String?, timeoutMs: Int?): UiWaitResult {
        if (until !in setOf("idle", "window", "text") || (until != "idle" && value == null)) throw DeviceRpcException(-32602, "Invalid wait condition")
        val timeout = timeoutMs ?: 5000
        if (timeout !in 1..60000) throw DeviceRpcException(-32602, "Invalid wait timeout")
        val deadline = SystemClock.uptimeMillis() + timeout
        do {
            val root = root()
            val matched = try {
                when (until) {
                    "idle" -> SystemClock.uptimeMillis() - AgentRuntime.lastUiEvent >= 300
                    "window" -> root.packageName?.toString() == value
                    else -> root.findAccessibilityNodeInfosByText(value).let { nodes ->
                        val found = nodes.isNotEmpty(); nodes.forEach { it.recycle() }; found
                    }
                }
            } finally { root.recycle() }
            if (matched) return UiWaitResult(true)
            delay(50)
        } while (SystemClock.uptimeMillis() < deadline)
        throw DeviceRpcException(2004, "Wait timed out")
    }

    override suspend fun uiScreenshot(maxWidth: Int?): UiScreenshotInfo {
        val initialRoot = root()
        val windowId = initialRoot.windowId
        initialRoot.recycle()
        if (Build.VERSION.SDK_INT < 30) throw DeviceRpcException(2002, "Screenshots require Android 11 or later")
        if (maxWidth != null && maxWidth !in 1..4096) throw DeviceRpcException(-32602, "Invalid screenshot max_width")
        val frame = currentCoroutineContext()[RequestFrame] ?: throw DeviceRpcException(-32603, "Screenshot has no request context")
        val bitmap: Bitmap = withTimeout(10000) {
            suspendCancellableCoroutine { continuation ->
                val callback = object : AccessibilityService.TakeScreenshotCallback {
                    override fun onSuccess(screenshot: AccessibilityService.ScreenshotResult) {
                        try {
                            val hardware = Bitmap.wrapHardwareBuffer(screenshot.hardwareBuffer, screenshot.colorSpace)
                            val copy = try { hardware?.copy(Bitmap.Config.ARGB_8888, false) } finally { hardware?.recycle() }
                            if (copy == null) { if (continuation.isActive) continuation.resumeWithException(DeviceRpcException(-32603, "Cannot copy screenshot")) }
                            else if (continuation.isActive) continuation.resume(copy) else copy.recycle()
                        } catch (_: Exception) {
                            if (continuation.isActive) continuation.resumeWithException(DeviceRpcException(-32603, "Cannot copy screenshot"))
                        } finally { screenshot.hardwareBuffer.close() }
                    }
                    override fun onFailure(errorCode: Int) {
                        if (continuation.isActive) continuation.resumeWithException(DeviceRpcException(if (errorCode == 6) 2003 else 2002, "Screenshot unavailable ($errorCode)"))
                    }
                }
                if (Build.VERSION.SDK_INT >= 34) service().takeScreenshotOfWindow(windowId, context.mainExecutor, callback)
                else service().takeScreenshot(Display.DEFAULT_DISPLAY, context.mainExecutor, callback)
            }
        }
        try {
            val current = root()
            try { if (current.windowId != windowId) throw DeviceRpcException(2003, "Window changed during screenshot") }
            finally { current.recycle() }
        } catch (error: Exception) { bitmap.recycle(); throw error }
        val output = if (maxWidth != null && bitmap.width > maxWidth) Bitmap.createScaledBitmap(bitmap, maxWidth, maxOf(1, bitmap.height * maxWidth / bitmap.width), true) else bitmap
        try {
            val bytes = ByteArrayOutputStream().also { output.compress(Bitmap.CompressFormat.PNG, 100, it) }.toByteArray()
            if (bytes.size > 8 * 1024 * 1024) throw DeviceRpcException(2002, "Screenshot exceeds transport limit")
            frame.binary = bytes
            return UiScreenshotInfo("image/png", output.width, output.height, bytes.size)
        } finally { if (output !== bitmap) output.recycle(); bitmap.recycle() }
    }

    override suspend fun notificationList(limit: Int?): NotificationListResult {
        val count = limit ?: 100
        if (count !in 1..1000) throw DeviceRpcException(-32602, "Invalid notification limit")
        return NotificationListResult((AgentRuntime.notifications ?: throw DeviceRpcException(2005, "Notification access is not enabled")).list(count))
    }

    override suspend fun notificationReply(key: String, text: String, actionIndex: Int?): NotificationReplyResult {
        (AgentRuntime.notifications ?: throw DeviceRpcException(2005, "Notification access is not enabled")).reply(key, text, actionIndex)
        return NotificationReplyResult(true)
    }

    private companion object {
        val actions = mapOf("click" to AccessibilityNodeInfo.ACTION_CLICK, "long_click" to AccessibilityNodeInfo.ACTION_LONG_CLICK,
            "set_text" to AccessibilityNodeInfo.ACTION_SET_TEXT, "focus" to AccessibilityNodeInfo.ACTION_FOCUS,
            "scroll_forward" to AccessibilityNodeInfo.ACTION_SCROLL_FORWARD, "scroll_backward" to AccessibilityNodeInfo.ACTION_SCROLL_BACKWARD,
            "select" to AccessibilityNodeInfo.ACTION_SELECT)
    }
}
