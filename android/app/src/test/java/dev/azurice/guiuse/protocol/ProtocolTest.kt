package dev.azurice.guiuse.protocol

import kotlin.coroutines.Continuation
import kotlin.coroutines.EmptyCoroutineContext
import kotlin.coroutines.startCoroutine
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.decodeFromJsonElement
import kotlinx.serialization.json.encodeToJsonElement
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class ProtocolTest {
    private val json = Json

    @Test
    fun dispatchUnpacksNamedArgumentsAndDefaultsMissingOptionals() {
        val device = TestDevice()
        val result = immediate {
            ProtocolDispatch.dispatch(
                "app.launch",
                json.parseToJsonElement("""{"package_name":"com.example.app"}"""),
                json,
                device,
            )
        }
        assertEquals("com.example.app", device.packageName)
        assertEquals(json.parseToJsonElement("""{"accepted":true}"""), result)
        assertEquals(null, device.activity)
        assertEquals(null, device.forceStop)
    }

    @Test
    fun generatedOptionalParametersAreUsableThroughTheInterface() {
        val device: Device = TestDevice()
        val result = immediate { device.appList() }
        assertEquals("Example", result.apps.single().label)
    }

    @Test
    fun nodeAndNestedTypesKeepTheirRustWireFieldNames() {
        val wire = json.parseToJsonElement("""{
            "id":"n1", "parent":null, "class_name":"android.widget.TextView",
            "text":"Hello", "bounds":{"left":0,"top":1,"right":2,"bottom":3},
            "flags":{"clickable":true}, "actions":["click"]
        }""")
        val node = json.decodeFromJsonElement<Node>(wire)
        assertEquals("android.widget.TextView", node.class_name)
        assertEquals(3, node.bounds.bottom)
        assertEquals(true, node.flags.clickable)
        assertEquals(null, node.desc)
        val encoded = json.encodeToJsonElement(node).toString()
        assertTrue(encoded.contains("\"class_name\""))
        assertFalse(encoded.contains("className"))
    }

    @Test
    fun eventHelpersProduceJsonRpcNotificationsWithoutRequestIds() {
        val event = ProtocolEvents.notificationRemoved(
            43, "2026-11-12T01:23:45.678Z", NotificationRemovedData("notification-key"), json,
        )
        assertEquals(json.parseToJsonElement("""{
            "jsonrpc":"2.0", "method":"events", "params":{
                "seq":43, "ts":"2026-11-12T01:23:45.678Z",
                "type":"notification.removed", "data":{"key":"notification-key"}
            }
        }"""), event)
        assertFalse(event.containsKey("id"))
    }

    @Test
    fun dispatchRejectsMissingRequiredArgumentsAndUnknownMethods() {
        for ((method, params) in listOf("app.launch" to "{}", "missing" to "{}")) {
            assertTrue(runCatching {
                immediate { ProtocolDispatch.dispatch(method, json.parseToJsonElement(params), json, TestDevice()) }
            }.isFailure)
        }
    }

    // These handlers complete synchronously, so tests need no coroutine runtime.
    private fun <T> immediate(block: suspend () -> T): T {
        var outcome: Result<T>? = null
        block.startCoroutine(object : Continuation<T> {
            override val context = EmptyCoroutineContext
            override fun resumeWith(result: Result<T>) { outcome = result }
        })
        return checkNotNull(outcome).getOrThrow()
    }

    private class TestDevice : Device {
        var packageName: String? = null
        var activity: String? = null
        var forceStop: Boolean? = null

        override suspend fun appList(includeSystem: Boolean?): AppListResult =
            AppListResult(listOf(AppInfo("com.example.app", "Example", true)))

        override suspend fun appLaunch(packageName: String, activity: String?, forceStop: Boolean?): AppLaunchResult {
            this.packageName = packageName
            this.activity = activity
            this.forceStop = forceStop
            return AppLaunchResult(true)
        }

        override suspend fun uiSnapshot(filter: String?, maxNodes: Int?): UiSnapshotResult = error("unused")
        override suspend fun uiAct(node: String, action: String, text: String?): UiActResult = error("unused")
        override suspend fun uiGesture(strokes: List<GestureStroke>, timeoutMs: Int?): UiGestureResult = error("unused")
        override suspend fun uiWait(until: String, value: String?, timeoutMs: Int?): UiWaitResult = error("unused")
        override suspend fun uiScreenshot(maxWidth: Int?): UiScreenshotInfo = error("unused")
        override suspend fun notificationList(limit: Int?): NotificationListResult = error("unused")
        override suspend fun notificationReply(key: String, text: String, actionIndex: Int?): NotificationReplyResult = error("unused")
    }
}
