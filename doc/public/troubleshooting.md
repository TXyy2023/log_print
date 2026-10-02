<span id="排查常见问题"></span>

# Troubleshooting

| Symptom | Check and action |
| --- | --- |
| Configuration reports unknown field / missing role | Follow the [configuration reference](reference/configuration.md); old save fields and multi-stream Inputs are rejected |
| Stream not found | Run `streams` for the current UUID; identities change after Core restarts |
| A late Output misses the beginning | The bounded buffer may have overwritten it; Core has no disk history to recover |
| UDP was sent but no data appears | Local send success does not guarantee reception; check registration, encoded packet size and plugin errors |
| Editing config changes nothing | The running instance uses its startup snapshot; stop and restart the main program |
| Output fails to start | Check reads, UUIDs, destination paths, `status` and stderr; existing archive targets are never overwritten |
| tmux attachment is refused | Check for an existing pipe-pane; do not take over another collector's pipe |
| Plugin shutdown reports forced=true | Cleanup did not finish normally; inspect errors and saved data before claiming completeness |
| WebUI/TUI cannot find older records | Check archive coverage; without an archive, history is limited to retained Core memory |
| A Page edit reports a revision conflict | Refresh committed state and retry the intended edit |

`start` returns stdout/stderr log paths; `status` returns process and business state. State files contain management credentials: do not publish them or delete an active instance's file to bypass a conflict.
