"""Managed capture lifetime for read-only viewers on the pinned Selkies server."""

def retain_primary_for_viewers(server_class):
    original = server_class._reconfigure_displays_locked

    async def reconfigure(server):
        # A temporary kernel controller may release display ownership after
        # resize. It must not stop a capture that normal viewers still consume.
        # Viewer permissions are unchanged: this supplies no resize/input path.
        if (not server.display_clients and "primary" in server.capture_instances
                and server._active_primary_consumers()):
            for display_id in list(server.capture_instances):
                if display_id != "primary":
                    await server._stop_capture_for_display(display_id)
            return
        # Upstream retains its normal last/paused-viewer teardown and layout.
        await original(server)

    server_class._reconfigure_displays_locked = reconfigure


if __name__ == "__main__":
    from selkies.selkies import DataStreamingServer
    retain_primary_for_viewers(DataStreamingServer)
    from selkies.__main__ import main
    main()
