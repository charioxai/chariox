// MP-08/MP-10/MP-11: link scoped inputs through the product before Slice import.
import { requireScopedProviderPath } from "./private-drill-runtime.mjs"
import {getProviderAuthStatusRequest,linkProviderAccountProfileRequest,setDefaultProviderAccountProfileRequest,listProviderAccountProfilesRequest} from "../../dist/ipc-requests.js"
const scopes={codex:"CODEX_HOME",claude:"CLAUDE_CONFIG_DIR",opencode:"OPENCODE_CONFIG_DIR"}
export async function linkManagedSliceProviderProfile(client,provider,environment){
 const scopeName=scopes[provider]
 if(!scopeName)throw new Error("unsupported managed Slice provider")
 let profile
 if(environment[scopeName]?.trim()){
  const scope=requireScopedProviderPath(environment,scopeName)
  const linked=await client.send(linkProviderAccountProfileRequest(provider,`native Slice ${provider}`,scope))
  profile=linked.ProviderAccountProfile?.profile
 }else{
  const listed=await client.send(listProviderAccountProfilesRequest(provider))
  profile=listed.ProviderAccountProfilesListed?.profiles.find(value=>value.is_default&&value.auth_state==="authenticated")
  if(!profile)throw new Error(`explicit linked provider scope ${scopeName} or normal product enrollment is required`)
 }
 if(!profile?.profile_id)throw new Error("managed Slice provider product link failed")
 const auth=await client.send(getProviderAuthStatusRequest(provider,profile.profile_id))
 if(auth.ProviderAuthStatus?.status.auth_state!=="authenticated")throw new Error("managed Slice account requires normal product enrollment before this drill")
 const selected=await client.send(setDefaultProviderAccountProfileRequest(provider,profile.profile_id))
 if(selected.Error)throw new Error("managed Slice provider product selection failed")
 return profile
}
