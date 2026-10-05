"""MP-08 / MP-10: passive observations before official WebMall validation."""


def fresh_existing_pages(context, strict=True, on_error=None):
    """Pump external page events before consulting cached URL/context state.

    Read only. The caller retains official evaluation and first-done freezing.
    A failed observation propagates instead of certifying a stale final grade.
    """
    observed = []
    for ordinal, page in enumerate(context.pages):
        try:
            page.title()
        except Exception as error:
            if on_error is not None:
                on_error({"phase": "passive_observation", "pageOrdinal": ordinal,
                          "errorClass": type(error).__name__})
            if strict:
                raise
            continue  # A transient page cannot be graded using its cached URL.
        if not page.url.startswith("chrome-extension:"):
            observed.append(page)
    return observed
