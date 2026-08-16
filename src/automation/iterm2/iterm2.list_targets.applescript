use framework "Foundation"

set targetRecords to current application's NSMutableArray's array()

tell application "iTerm2"
    repeat with windowIndex from 1 to count of windows
        set itermWindow to item windowIndex of windows

        repeat with tabIndex from 1 to count of tabs of itermWindow
            set itermTab to item tabIndex of tabs of itermWindow

            repeat with terminalIndex from 1 to count of sessions of itermTab
                set itermSession to item terminalIndex of sessions of itermTab
                set terminalID to (unique ID of itermSession) as text
                set terminalName to (name of itermSession) as text
                tell itermSession
                    set terminalDirectory to (variable named "path") as text
                end tell

                set targetRecord to current application's NSMutableDictionary's dictionary()
                targetRecord's setObject:terminalID forKey:"id"
                targetRecord's setObject:terminalName forKey:"name"
                targetRecord's setObject:terminalDirectory forKey:"working_directory"
                targetRecord's setObject:windowIndex forKey:"window_index"
                targetRecord's setObject:tabIndex forKey:"tab_index"
                targetRecord's setObject:terminalIndex forKey:"terminal_index"
                targetRecords's addObject:targetRecord
            end repeat
        end repeat
    end repeat
end tell

set jsonData to current application's NSJSONSerialization's dataWithJSONObject:targetRecords options:0 |error|:(missing value)
set jsonText to current application's NSString's alloc()'s initWithData:jsonData encoding:(current application's NSUTF8StringEncoding)

return jsonText as text
