"""Scenario: Telegram bot token validation - configure modal UI test.

Tests the Telegram extension configure modal renders and accepts tokens with colons.

Note: The core URL-building logic (colon preservation, no %3A encoding) is verified
by unit tests in src/extensions/manager.rs. This E2E test verifies the configure modal
UI can accept Telegram tokens with colons and renders correctly.
"""

import json

from helpers import SEL, api_get


# ─── Fixture data ─────────────────────────────────────────────────────────────

_TELEGRAM_EXTENSION = {
    "name": "telegram",
    "display_name": "Telegram",
    "kind": "wasm_channel",
    "description": "Telegram bot channel",
    "url": None,
    "active": False,
    "authenticated": False,
    "has_auth": True,
    "needs_setup": True,
    "tools": [],
    "activation_status": "installed",
    "activation_error": None,
}

_TELEGRAM_SECRETS = [
    {
        "name": "telegram_bot_token",
        "prompt": "Telegram Bot Token",
        "provided": False,
        "optional": False,
        "auto_generate": False,
    }
]


# ─── Tests ────────────────────────────────────────────────────────────────────

async def test_telegram_configure_modal_renders(page):
    """
    Telegram extension configure modal renders with correct fields.

    Verifies that the configure modal appears with the Telegram bot token field
    and all expected UI elements are present.
    """
    ext_body = json.dumps({"extensions": [_TELEGRAM_EXTENSION]})

    async def handle_ext_list(route):
        if route.request.url.endswith("/api/extensions"):
            await route.fulfill(
                status=200, content_type="application/json", body=ext_body
            )
        else:
            await route.continue_()

    await page.route("**/api/extensions*", handle_ext_list)

    async def handle_setup(route):
        if route.request.method == "GET":
            await route.fulfill(
                status=200,
                content_type="application/json",
                body=json.dumps({"secrets": _TELEGRAM_SECRETS}),
            )
        else:
            await route.continue_()

    await page.route("**/api/extensions/telegram/setup", handle_setup)
    await page.evaluate("showConfigureModal('telegram')")
    modal = page.locator(SEL["configure_modal"])
    await modal.wait_for(state="visible", timeout=5000)

    # Modal should contain the extension name and token prompt
    modal_text = await modal.text_content()
    assert "telegram" in modal_text.lower()
    assert "bot token" in modal_text.lower()

    # Input field should be present
    input_field = page.locator(SEL["configure_input"])
    assert await input_field.is_visible()


async def test_telegram_token_input_accepts_colon_format(page):
    """
    Telegram bot token input accepts tokens with colon separator.

    Verifies that a token in the format `numeric_id:alphanumeric_string`
    can be entered without browser-side validation errors.
    """
    ext_body = json.dumps({"extensions": [_TELEGRAM_EXTENSION]})

    async def handle_ext_list(route):
        if route.request.url.endswith("/api/extensions"):
            await route.fulfill(
                status=200, content_type="application/json", body=ext_body
            )
        else:
            await route.continue_()

    await page.route("**/api/extensions*", handle_ext_list)

    async def handle_setup(route):
        if route.request.method == "GET":
            await route.fulfill(
                status=200,
                content_type="application/json",
                body=json.dumps({"secrets": _TELEGRAM_SECRETS}),
            )

    await page.route("**/api/extensions/telegram/setup", handle_setup)
    await page.evaluate("showConfigureModal('telegram')")
    await page.locator(SEL["configure_modal"]).wait_for(state="visible", timeout=5000)

    # Enter a valid Telegram bot token with colon
    token_value = "123456789:AABBccDDeeFFgg_Test-Token"
    input_field = page.locator(SEL["configure_input"])
    await input_field.fill(token_value)

    # Verify the value was entered and colon is preserved
    entered_value = await input_field.input_value()
    assert entered_value == token_value
    assert ":" in entered_value, "Colon should be preserved in token"
    assert "%3A" not in entered_value, "Colon should not be URL-encoded in input"


async def test_telegram_token_with_underscores_and_hyphens(page):
    """
    Telegram tokens with hyphens and underscores are accepted.

    Verifies that valid Telegram token characters (hyphens, underscores) are
    properly accepted by the input field.
    """
    ext_body = json.dumps({"extensions": [_TELEGRAM_EXTENSION]})

    async def handle_ext_list(route):
        if route.request.url.endswith("/api/extensions"):
            await route.fulfill(
                status=200, content_type="application/json", body=ext_body
            )
        else:
            await route.continue_()

    await page.route("**/api/extensions*", handle_ext_list)

    async def handle_setup(route):
        if route.request.method == "GET":
            await route.fulfill(
                status=200,
                content_type="application/json",
                body=json.dumps({"secrets": _TELEGRAM_SECRETS}),
            )

    await page.route("**/api/extensions/telegram/setup", handle_setup)
    await page.evaluate("showConfigureModal('telegram')")
    await page.locator(SEL["configure_modal"]).wait_for(state="visible", timeout=5000)

    # Token with hyphens and underscores
    token_value = "987654321:ABCD-EFgh_ijkl-MNOP_qrst"
    input_field = page.locator(SEL["configure_input"])
    await input_field.fill(token_value)

    # Verify the value was entered correctly with all characters preserved
    entered_value = await input_field.input_value()
    assert entered_value == token_value
    assert "-" in entered_value
    assert "_" in entered_value


async def test_pasted_telegram_token_redirects_to_secure_setup_without_history_leak(
    page, ironclaw_server
):
    """Pasting a Telegram bot token into chat opens secure setup and avoids history leaks."""
    setup_requests = []

    async def handle_setup(route):
        setup_requests.append(route.request.method)
        await route.fulfill(
            status=200,
            content_type="application/json",
            body=json.dumps({"secrets": _TELEGRAM_SECRETS}),
        )

    await page.route("**/api/extensions/telegram/setup", handle_setup)

    token_value = "123456789:AABBccDDeeFFgg_Test-Token"
    chat_input = page.locator(SEL["chat_input"])
    await chat_input.wait_for(state="visible", timeout=5000)
    await chat_input.fill(token_value)
    await chat_input.press("Enter")

    modal = page.locator(SEL["configure_modal"])
    await modal.wait_for(state="visible", timeout=8000)
    assert setup_requests, "Telegram secure setup modal was not requested"

    user_msg = page.locator(SEL["message_user"]).last
    await user_msg.wait_for(state="visible", timeout=5000)
    user_text = await user_msg.text_content()
    assert "redacted" in user_text.lower()
    assert token_value not in user_text

    assistant_msg = page.locator(SEL["message_assistant"]).last
    await assistant_msg.wait_for(state="visible", timeout=8000)
    assistant_text = await assistant_msg.text_content()
    assert "secure telegram setup flow" in assistant_text.lower()

    thread_id = await page.evaluate("currentThreadId")
    history = await api_get(
        ironclaw_server,
        f"/api/chat/history?limit=50&thread_id={thread_id}",
    )
    assert history.status_code == 200
    history_data = history.json()
    assert token_value not in json.dumps(history_data)
    assert history_data["turns"] == [], history_data

    await page.evaluate("loadHistory()")
    await page.wait_for_function(
        """token => {
            const chat = document.querySelector('#chat-messages');
            return !!chat && !(chat.innerText || '').includes(token);
        }""",
        token_value,
        timeout=8000,
    )
