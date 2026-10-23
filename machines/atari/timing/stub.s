; stub.s: the boot-sector program every timing probe shares (at most 480 bytes, position independent).
; It reads the probe body from cylinders 1.. of side 0 (9 sectors a track) into $90000 with the XBIOS Floprd call, which names
; the cylinder, side and sector itself (a BIOS Rwabs would map logical sectors through TOS's own geometry), and jumps
; there. mkprobe.py writes the probe body to those tracks, so a probe may be up to 32256 bytes. EmuTOS reads one sector per disk revolution (about 0.2 s), so a
; track costs roughly 2 seconds of machine time: keep probes short.
    .org 0
    lea     $90000,a3
    moveq   #1,d3               ; the cylinder
next:
    move.w  #9,-(sp)            ; sectors
    move.w  #0,-(sp)            ; side
    move.w  d3,-(sp)            ; cylinder
    move.w  #1,-(sp)            ; first sector
    move.w  #0,-(sp)            ; drive A
    clr.l   -(sp)               ; filler
    move.l  a3,-(sp)            ; buffer
    move.w  #8,-(sp)            ; Floprd
    trap    #14
    lea     20(sp),sp
    lea     4608(a3),a3
    addq.w  #1,d3
    cmp.w   #@END@,d3           ; mkprobe.py puts 1 + the number of tracks of the body here
    bne.s   next
    jmp     $90000
