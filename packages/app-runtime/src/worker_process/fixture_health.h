/* Test-only health/startup behavior on the actual inherited SDK channel. */
static int fixture_health_mode(const char* mode) {
  return !strcmp(mode, "sdk_health") || !strcmp(mode, "sdk_bad_health");
}
static int fixture_health_parse(char response[8193], const char* event, char id[129]) {
  if (strncmp(response, "{\"kind\":\"request\",", 18) ||
      !strstr(response, "\"method\":\"lifecycle.dispatch\"")) return -1;
  char expected[128];
  const int count=snprintf(expected,sizeof(expected),"\"event\":\"%s\"",event);
  if (count<0 || (size_t)count>=sizeof(expected) || !strstr(response,expected)) return -1;
  const char* begin=strstr(response,"\"id\":\"");
  if (!begin) return -1;
  begin+=6;
  const char* end=strchr(begin,'"');
  if (!end || end==begin || end-begin>128) return -1;
  for (const char* p=begin;p!=end;++p) {
    if (!((*p>='a'&&*p<='z')||(*p>='A'&&*p<='Z')||(*p>='0'&&*p<='9')||
        *p=='-'||*p=='_'||*p=='.'||*p==':')) return -1;
  }
  memcpy(id,begin,(size_t)(end-begin));id[end-begin]=0;
  return 1;
}
static int fixture_health_id(char response[8193], const char* event, char id[129]) {
  if (fixture_sdk_receive(response,cx_monotonic_ms()+(!strcmp(event,"shutdown") ? 30000 : 5000))!=1) return -1;
  return fixture_health_parse(response,event,id);
}
static int fixture_health_reply(const char* id, int failed) {
  char response[512];
  const char* result=failed ? "\"error\":{\"code\":\"HEALTH_FAILED\",\"message\":\"fixture health failure\"}" : "\"result\":null";
  const int count=snprintf(response,sizeof(response),
      "{\"kind\":\"response\",\"version\":1,\"generation\":\"1\",\"id\":\"%s\",%s}",id,result);
  if (count<0 || (size_t)count>=sizeof(response)) return -1;
  return fixture_sdk_send(response);
}
static int fixture_health_before(const char* mode, char response[8193]) {
  char id[129];
  if (fixture_health_id(response,"health_check",id)!=1) return -1;
  if (fixture_sdk_request("during-health","state.get","{\"key\":\"fixture\"}")!=1 ||
      fixture_sdk_receive(response,cx_monotonic_ms()+5000)!=1 ||
      !strstr(response,"\"id\":\"during-health\"") || !strstr(response,"\"code\":\"APP_NOT_READY\"")) return -1;
  return fixture_health_reply(id,!strcmp(mode,"sdk_bad_health"));
}
static int fixture_health_after(char response[8193], char startup[8193]) {
  char id[129];
  if ((startup[0] ? fixture_health_parse(startup,"startup",id) : fixture_health_id(response,"startup",id))!=1) return -1;
  if (fixture_sdk_request("startup-write","files.atomic_replace",
      "{\"path\":\"fixture-file\",\"contentsBase64\":\"c3RhcnR1cA==\"}")!=1 ||
      fixture_sdk_receive(response,cx_monotonic_ms()+5000)!=1 ||
      strcmp(response,"{\"kind\":\"response\",\"version\":1,\"generation\":\"1\",\"id\":\"startup-write\",\"result\":{\"bytesWritten\":7}}")) return -1;
  if (fixture_health_reply(id,0)!=1 || fixture_health_id(response,"shutdown",id)!=1) return -1;
  return fixture_health_reply(id,0);
}
