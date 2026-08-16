use framework "Foundation"

set targetRecords to current application's NSMutableArray's array()

tell application "Ghostty"
    repeat with windowIndex from 1 to count of windows
        set ghosttyWindow to item windowIndex of windows

        repeat with tabIndex from 1 to count of tabs of ghosttyWindow
            set ghosttyTab to item tabIndex of tabs of ghosttyWindow

            repeat with terminalIndex from 1 to count of terminals of ghosttyTab
                set ghosttyTerminal to item terminalIndex of terminals of ghosttyTab
                set terminalID to (id of ghosttyTerminal) as text
                set terminalName to (name of ghosttyTerminal) as text
                set terminalDirectory to (working directory of ghosttyTerminal) as text

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
