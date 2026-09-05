package io.mirelay.android

import org.junit.Assert.*
import org.junit.Test

class InputRulesTest {
    @Test fun productionRequiresHttpsEvenWhenInsecureFlagIsSet() {
        assertThrows(IllegalArgumentException::class.java) { InputRules.server("http://localhost:8080", true, false) }
        assertThrows(IllegalArgumentException::class.java) { InputRules.server("http://localhost:8080", false, true) }
        assertEquals("http://localhost:8080/prefix", InputRules.server(" http://localhost:8080/prefix/ ", true, true))
    }
    @Test fun urlsRejectCredentialsQueriesFragmentsAndMissingHosts() {
        for (url in listOf("https://user:password@example.com", "https://example.com?token=x", "https://example.com#x", "file:///tmp", "https:///missing", "https://example.com:99999")) {
            assertThrows(IllegalArgumentException::class.java) { InputRules.server(url, false, false) }
        }
        assertEquals("https://example.com/base", InputRules.server("https://example.com/base/", false, false))
    }
    @Test fun tokenValidationRejectsControlCharactersAndOversizeBytes() {
        for (token in listOf("", "  ", "bad\ntoken", "x".repeat(4097), "中".repeat(1366))) {
            assertThrows(IllegalArgumentException::class.java) { InputRules.token(token) }
        }
        assertEquals("secret", InputRules.token(" secret "))
    }
    @Test fun filenamesCannotEscapeStagingOrContainControls() {
        for (name in listOf("../a/b", "a\\b", "x\u0000y", "x\ny")) {
            val safe = InputRules.filename(name)
            assertFalse(safe.contains('/')); assertFalse(safe.contains('\\'))
            assertFalse(safe.any { it.isISOControl() })
        }
        for (name in listOf(null, "", ".", "..", "   ")) assertEquals("Shared file", InputRules.filename(name))
    }
    @Test fun unicodeNamesStayValidAndWithinServerByteLimit() {
        val name = InputRules.filename("插画😀".repeat(100))
        assertTrue(name.toByteArray(Charsets.UTF_8).size <= 255)
        assertFalse(name.contains('\uFFFD'))
        assertEquals(name, name.toByteArray(Charsets.UTF_8).toString(Charsets.UTF_8))
    }
}
