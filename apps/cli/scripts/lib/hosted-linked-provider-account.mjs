// MP-08 / MP-10 / MP-11: profiles stay on the execution kernel. Linking
// registers a credential scope; the official status probe verifies admission.
export async function prepareHostedLinkedProviderAccount({ client, requests, provider, label, path }) {
  const linked = profileFrom(await client.send(
    requests.linkProviderAccountProfileRequest(provider, label, path),
  ))
  const verified = profileFrom(await client.send(
    requests.refreshProviderAccountProfileRequest(provider, linked.profile_id),
  ))
  if (verified.profile_id !== linked.profile_id || verified.provider !== provider) {
    throw new Error("linked provider verification returned another account")
  }
  if (verified.auth_state !== "authenticated") {
    throw new Error(`linked ${provider} account is not authenticated; verify it in Provider Accounts`)
  }
  return verified
}

function profileFrom(response) {
  if (response.Error) throw new Error(response.Error.message)
  const profile = response.ProviderAccountProfile?.profile
  if (!profile?.profile_id) throw new Error("kernel did not return a provider account profile")
  return profile
}
