/* Test-only inbox App: the fixture supplies a complete deterministic occurrence
 * in metadata. Route background attribution selects exactly one automation.
 * No package-selected code or production launch path uses this handler. */
static int fixture_inbox_emit(char response[8193]) {
  const char* field = strstr(response,"inbox:second") ? "\"emissionSecond\":" : "\"emissionFirst\":";
  const char* start = strstr(response,field);
  if (!start) return 0;
  start += strlen(field);
  if (*start!='{') return 0;
  const char* end=start;
  unsigned depth=0; int quoted=0,escaped=0;
  do {
    char c=*end++;
    if (!c) return 0;
    if (quoted) {
      if (escaped) escaped=0;
      else if (c=='\\') escaped=1;
      else if (c=='"') quoted=0;
    } else if (c=='"') quoted=1;
    else if (c=='{') depth++;
    else if (c=='}') depth--;
  } while (depth);
  size_t length=(size_t)(end-start);
  char emission[8193];
  if (length>=sizeof(emission)) return 0;
  memcpy(emission,start,length); emission[length]=0;
  return fixture_sdk_request("inbox-emit","events.emit",emission)==1 &&
      fixture_sdk_receive(response,cx_monotonic_ms()+5000)==1 &&
      strstr(response,"\"id\":\"inbox-emit\",\"result\":");
}

/* Fixed lifecycle observer. Never linked into production or package-selected. */
static int fixture_lifecycle_run(const struct cx_launch_record* record, const char* mode,
    char response[8193], char startup[8193]) {
  char path[1200];
  int count=snprintf(path,sizeof(path),"%s/lifecycle-frames",record->roots[CX_DATA]);
  if (count<0 || (size_t)count>=sizeof(path)) return 140;
  int file=open(path,O_WRONLY|O_CREAT|O_APPEND,0600);
  if (file<0) return 141;
  const char* names[]={"startup","suspend","resume","prepare_update","configuration_change","shutdown"};
  int buffered=startup[0]!=0;
  for (;;) {
    if (buffered) { memcpy(response,startup,strlen(startup)+1); buffered=0; }
    else {
      int received=fixture_sdk_receive(response,cx_monotonic_ms()+120000);
      if (received==0) { close(file); return 0; }
      if (received!=1) { close(file); return 142; }
    }
    if (strstr(response,"\"kind\":\"cancel\"") || strstr(response,"\"kind\":\"event\"")) continue;
    char id[129]; const char* event=NULL;
    for (size_t i=0;i<sizeof(names)/sizeof(names[0]);++i)
      if (fixture_health_parse(response,names[i],id)==1) { event=names[i]; break; }
    if (!event && strstr(response,"\"method\":\"schedule.wake\"") &&
        fixture_request_id(response,id)==1) event="wake";
    if (!event && (!strcmp(mode,"sdk_lifecycle_inbox") || !strcmp(mode,"sdk_lifecycle_inbox_automation")) &&
        strstr(response,"\"method\":\"events.deliver\"") &&
        fixture_request_id(response,id)==1) event="incoming";
    if (!event) { close(file); return 143; }
    size_t size=strlen(response);
    if (write(file,response,size)!=(ssize_t)size || write(file,"\n",1)!=1 || fsync(file)) { close(file); return 144; }
    // A hanging callback never replies, even to cancellation. A later
    // lifecycle frame is still recorded so tests detect an illegal overlap.
    if (!strncmp(mode,"sdk_lifecycle_hang_",19) && !strcmp(mode+19,event)) continue;
    if (!strcmp(event,"prepare_update") && !strcmp(mode,"sdk_lifecycle")) {
      if (fixture_sdk_request("prepare-write","state.transaction",
          "{\"schemaVersion\":0,\"checks\":[],\"writes\":[{\"key\":\"prepared\",\"value\":true}]}")!=1 ||
          fixture_sdk_receive(response,cx_monotonic_ms()+5000)!=1 ||
          !strstr(response,"\"id\":\"prepare-write\",\"result\":")) { close(file); return 145; }
    }
    if (!strcmp(mode,"sdk_lifecycle_slow_resume") && !strcmp(event,"resume")) {
      struct timespec remaining={21,0};
      while (nanosleep(&remaining,&remaining)<0 && errno==EINTR) {}
    }
    if (!strcmp(event,"incoming") && !strcmp(mode,"sdk_lifecycle_inbox_automation") &&
        !fixture_inbox_emit(response)) { close(file); return 148; }
    if (fixture_health_reply(id,(!strcmp(mode,"sdk_lifecycle_fail_prepare") && !strcmp(event,"prepare_update")) ||
        (!strcmp(mode,"sdk_lifecycle_fail_configuration_change") && !strcmp(event,"configuration_change")))!=1) {
      close(file); return 146;
    }
    if (!strcmp(event,"incoming") && strstr(response,"\"text\":\"crash\"")) {
      // Let the acknowledgement settle before the deliberate native crash.
      struct timespec remaining={0,100000000};
      while (nanosleep(&remaining,&remaining)<0 && errno==EINTR) {}
      raise(SIGKILL);
    }
    if (!strcmp(event,"shutdown")) { close(file); return 0; }
  }
}
