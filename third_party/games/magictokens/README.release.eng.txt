Magic Tokens by Shiru (shiru@mail.ru) 08'10
For 'Your Game-4' competition


How to play

There are 64 tokens on the playfield. You should exchange two close tokens, making them into horizontal or vertical lines of three or more tokens of the same color. After you have made the line, the tokens are disappear from the playfield. To exchange the tokens, press and hold FIRE button on a token, then press a direction. If there is no line after the exchange, the tokens will return back to their original positions.

The game consists of ten levels. To complete a level you should remove a number of tokens with 'gems' at their center. On the higher levels there will be some bonuses, which appear when you make series of matched lines after one exchange.

The game supports Kempston, Sinclair Left and Right, Cursor joystics, and QAOPM keyboard. During the gameplay you can call a menu using Enter key. There you can restart current level or turn off the sound effects.

Please note that there are two different versions of the game. The original version had only one level of difficulty. Version 1.1 has choice of two levels of the difficulty, it was added by users request. HARD level is the level of the difficulty of the original version.


Development

The game was made in three and a half weeks. The goal was to try use C generally and SDCC compiler in an actual project, secondary goal was to make an entry for the 'Your Game' compo, if possible. The concept was chosen with two requirements: short development time, somewhat playable result. Sources of the inspiration for the game were Treasures of Montezuma for PC, and some other Bejeweled clones. From the beginning the game was designed in retro-style, for the original ZX Spectrum 48K.

Half of the code of the game is written in C, other half in assembly. Total count of lines in the code is 4500+. Initially I planned to use Beepola for the music, and the title song was made with it. Afterwards I decided to develop new beeper music engine 'Stocker', that took a few days. The song was then remade for the engine, and few jingles were made as well.

Recently there were a lot of discussions around the copyright in modern ZX Spectrum projects. For this reason the game is released into the public domain. You are completely free to copy, modify, distribute, sell, buy, play, and do whatever you want with it.

Thanks for the authors of all the software, which was used for development:

SDCC - C compiler
hex2bin - a tool to convert SDCC output into binary
bin2sna - a tool to create SNA from set of binaries, was used for debug builds
SjAsmPlus - part of the assembly code was developed and debugged with it first, and then converted into ASxxxx format; some other code was also compiled with it and then used as binary
DevCPP - was used to make a few custom tools
Notepad++ - for edit all the code
BASin - to prepare Basic loaders
GraphicsGale - main graphics editor
GIMP - secondary graphics editor, for some operations
SevenUp - was used as graphics converter for in-game graphics
BMP2Scr EXP - graphics converter for other graphics
BitBuster - some game resources compressed with it, and unpacked when needed
Hrust - to pack final binary of the game
Beepola - to create first version of the title song
Vortex Tracker II - to prepare all the music in the game, text files from the editor were then converted into music engine data with a custom tool
UnrealSpeccy - main emulator for all the debug and testing
SPIN - secondary emulator
Spectrum Navigator - to prepare TR-DOS version