// Human UI calls use the ambient bridge; the registered form keeps its caller.
const form = document.getElementById('automations')
const fields = [...form.querySelectorAll('input[name]')]

function show(automations) {
  for (const field of fields) {
    field.value = (automations[field.name] ?? []).join(', ')
    field.removeAttribute('aria-invalid')
  }
}

window.chariox.form(form, async (_input, caller) => {
  const automations = Object.fromEntries(fields.map(field => [
    field.name,
    field.value.split(',').map(name => name.trim()).filter(Boolean),
  ]))
  try {
    show((await caller.call('configure_automations', { automations })).automations)
  } catch {
    for (const field of fields) field.setAttribute('aria-invalid', 'true')
  }
})

window.chariox.call('get_automations', {}).then(saved => show(saved.automations), () => {})
