on run arguments
    if (count of arguments) is not 2 then
        error "usage: ghostty.send_text.applescript TERMINAL_ID TEXT"
    end if

    set targetID to item 1 of arguments
    set targetText to item 2 of arguments

    tell application "Ghostty"
        repeat with ghosttyWindow in windows
            repeat with ghosttyTab in tabs of ghosttyWindow
                repeat with ghosttyTerminal in terminals of ghosttyTab
                    if ((id of ghosttyTerminal) as text) is targetID then
                        input text targetText to ghosttyTerminal
                        return targetID
                    end if
                end repeat
            end repeat
        end repeat
    end tell

    error "Ghostty terminal not found: " & targetID
end run
