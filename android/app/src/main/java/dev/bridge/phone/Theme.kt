package dev.bridge.phone

import androidx.activity.ComponentActivity
import androidx.activity.SystemBarStyle
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext

/** The colors of the Android design (mockups E, F, G, N), light and dark. */
data class Palette(
    val bg: Color,
    val card: Color,
    val side: Color,
    val field: Color,
    val label: Color,
    val label2: Color,
    val sep: Color,
    val tint: Color,
    val tintText: Color,
    val green: Color,
    val red: Color,
    val chat: Color,
    val bubble: Color,
    val outline: Color,
)

val LightPalette = Palette(
    bg = Color(0xFFF2F2F7), card = Color(0xFFFFFFFF), side = Color(0xFFE6E6EC), field = Color(0xFFEFEFF4),
    label = Color(0xFF1C1C1E), label2 = Color(0xFF606064), sep = Color(0xFFE2E2E7),
    tint = Color(0xFF0A6CE8), tintText = Color(0xFF0A6CE8), green = Color(0xFF1F7A36), red = Color(0xFFD70015),
    chat = Color(0xFFEFEFF4), bubble = Color(0xFFFFFFFF), outline = Color(0xFF9A9AA1),
)

val DarkPalette = Palette(
    bg = Color(0xFF0E0E10), card = Color(0xFF1C1C1E), side = Color(0xFF161618), field = Color(0xFF2C2C2E),
    label = Color(0xFFF2F2F7), label2 = Color(0xFFA1A1A6), sep = Color(0xFF38383A),
    tint = Color(0xFF0A6CE8), tintText = Color(0xFF4DA3FF), green = Color(0xFF4CD27A), red = Color(0xFFFF6B6B),
    chat = Color(0xFF2C2C2E), bubble = Color(0xFF1C1C1E), outline = Color(0xFF636366),
)

val LocalPalette = staticCompositionLocalOf { LightPalette }

/** Which colors to use: what Android says (the default), or always light or dark. */
enum class ThemeMode { System, Light, Dark }

@Composable
fun BridgeTheme(content: @Composable () -> Unit) {
    val mode by Bridge.themeMode.collectAsState()
    val dark = when (mode) {
        ThemeMode.System -> isSystemInDarkTheme()
        ThemeMode.Light -> false
        ThemeMode.Dark -> true
    }
    val p = if (dark) DarkPalette else LightPalette
    val scheme = if (dark) {
        darkColorScheme(primary = p.tint, background = p.bg, surface = p.card, onSurface = p.label, onBackground = p.label)
    } else {
        lightColorScheme(primary = p.tint, background = p.bg, surface = p.card, onSurface = p.label, onBackground = p.label)
    }
    SystemBars(dark)
    CompositionLocalProvider(LocalPalette provides p) {
        MaterialTheme(colorScheme = scheme, content = content)
    }
}

/** Dark icons on the status bar in light mode and light icons in dark mode, whatever the phone itself is set to. */
@Composable
private fun SystemBars(dark: Boolean) {
    val activity = LocalContext.current as? ComponentActivity ?: return
    DisposableEffect(dark) {
        val clear = android.graphics.Color.TRANSPARENT
        activity.enableEdgeToEdge(
            statusBarStyle = SystemBarStyle.auto(clear, clear) { dark },
            navigationBarStyle = SystemBarStyle.auto(clear, clear) { dark },
        )
        onDispose {}
    }
}
