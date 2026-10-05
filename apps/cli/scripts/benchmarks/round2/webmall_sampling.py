"""MP-08 / MP-10: passive observations before official WebMall validation."""


def fresh_existing_pages(context):
    """Pump external page events before consulting cached URL/context state.

    Read only. The caller retains official evaluation and first-done freezing.
    A failed observation propagates instead of certifying a stale final grade.
    """
    for page in context.pages:
        page.title()
    return [page for page in context.pages if not page.url.startswith("chrome-extension:")]
