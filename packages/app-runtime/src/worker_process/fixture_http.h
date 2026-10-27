/* Fixed inherited-channel HTTP fixture. It talks to the test Echo transport;
 * this proves routing/ownership, not TLS, sandbox containment or Fetch support. */
static int fixture_http_reply(char response[8193], const char* id) {
  if (fixture_sdk_receive(response,cx_monotonic_ms()+5000)!=1) return -1;
  char expected[256];
  int count=snprintf(expected,sizeof(expected),"\"id\":\"%s\"",id);
  if (count<0 || (size_t)count>=sizeof(expected) || !strstr(response,expected) || !strstr(response,"\"result\":")) return -1;
  return 1;
}
static int fixture_http_run(char response[8193], int paused) {
  if (fixture_sdk_request("http-open","http.open","{\"url\":\"https://api.example.com/fixture\",\"method\":\"POST\",\"headers\":[],\"hasBody\":true}")!=1 || fixture_http_reply(response,"http-open")!=1) return 130;
  const char* start=strstr(response,"\"streamId\":\"");
  if (!start) return 131;
  start+=12;
  char stream[37];
  for (unsigned i=0;i<36;++i) {
    if (!((start[i]>='a'&&start[i]<='f')||(start[i]>='0'&&start[i]<='9')||start[i]=='-')) return 132;
    stream[i]=start[i];
  }
  if (start[36]!='"') return 133;
  stream[36]=0;
  char params[256];
  if (paused) {
    for (unsigned n=0;n<3;++n) {
      int count=snprintf(params,sizeof(params),"{\"streamId\":\"%s\",\"bodyBase64\":\"YWJj\",\"end\":false}",stream);
      if (count<0||(size_t)count>=sizeof(params)||fixture_sdk_request("http-blocked","http.write",params)!=1) return 144;
      if (n<2) {
        if (fixture_http_reply(response,"http-blocked")!=1||!strstr(response,"\"bytesWritten\":3")) return 145;
      } else {
        if (fixture_sdk_event("worker.fixture.http_blocked")!=1) return 146;
        /* Host teardown may close the channel or send cancellation before EOF. */
        int received=fixture_sdk_receive(response,cx_monotonic_ms()+5000);
        if (!received) return 0;
        if (received!=1||!strstr(response,"\"error\":")) return 147;
        return 0;
      }
    }
  }
  int count=snprintf(params,sizeof(params),"{\"streamId\":\"%s\",\"bodyBase64\":\"YWJj\",\"end\":true}",stream);
  if (count<0||(size_t)count>=sizeof(params)||fixture_sdk_request("http-write","http.write",params)!=1||fixture_http_reply(response,"http-write")!=1||!strstr(response,"\"bytesWritten\":3")) return 134;
  count=snprintf(params,sizeof(params),"{\"streamId\":\"%s\"}",stream);
  if (count<0||(size_t)count>=sizeof(params)) return 135;
  unsigned tries;
  for (tries=0;tries<5;++tries) {
    if (fixture_sdk_request("http-head","http.headers",params)!=1||fixture_http_reply(response,"http-head")!=1) return 136;
    if (!strstr(response,"\"pending\":true")) break;
  }
  if (tries==5||!strstr(response,"\"status\":200")) return 137;
  for (tries=0;tries<5;++tries) {
    if (fixture_sdk_request("http-read","http.read",params)!=1||fixture_http_reply(response,"http-read")!=1) return 138;
    if (!strstr(response,"\"pending\":true")) break;
  }
  if (tries==5||!strstr(response,"\"chunkBase64\":\"cmVwbHk=\"")||!strstr(response,"\"done\":false")) return 139;
  for (tries=0;tries<5;++tries) {
    if (fixture_sdk_request("http-end","http.read",params)!=1||fixture_http_reply(response,"http-end")!=1) return 140;
    if (!strstr(response,"\"pending\":true")) break;
  }
  if (tries==5||!strstr(response,"\"done\":true")) return 141;
  if (fixture_sdk_request("http-cancel","http.cancel",params)!=1||fixture_http_reply(response,"http-cancel")!=1||!strstr(response,"\"result\":null")) return 142;
  if (fixture_sdk_event("worker.fixture.http_complete")!=1) return 143;
  return 0;
}
