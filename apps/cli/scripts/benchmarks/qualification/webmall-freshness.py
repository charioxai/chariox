#!/usr/bin/env python3
"""MP-08 / MP-10 / MP-11 H6: read-only sampling before unchanged evaluation."""

def sample_existing_pages(context, evaluate):
    for page in context.pages:
        page.title()  # synchronous read pumps external CDP navigation events
        if page.url.startswith('chrome-extension:'):
            continue
        yield evaluate(page)


class FirstDoneGrade:
    """Harness latch; preserve the first official done verdict, including zero."""
    def __init__(self):
        self.result = None

    def sample(self, context, evaluator, checkpoints):
        if self.result is not None:
            return self.result
        for result in sample_existing_pages(context, lambda p: evaluator.eval('', p, checkpoints)):
            if evaluator.done:
                self.result = result
                return result
        return None
