"""MP-08 / MP-10: passive observations before official WebMall validation."""


def fresh_existing_pages(context):
    """Keep the historical sampling order until the external-CDP regression."""
    return [page for page in context.pages if not page.url.startswith("chrome-extension:")]
