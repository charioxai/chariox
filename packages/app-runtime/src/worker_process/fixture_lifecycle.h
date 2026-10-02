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
    if (fixture_health_reply(id,(!strcmp(mode,"sdk_lifecycle_fail_prepare") && !strcmp(event,"prepare_update")) ||
        (!strcmp(mode,"sdk_lifecycle_fail_configuration_change") && !strcmp(event,"configuration_change")))!=1) {
      close(file); return 146;
    }
    if (!strcmp(event,"shutdown")) { close(file); return 0; }
  }
}
