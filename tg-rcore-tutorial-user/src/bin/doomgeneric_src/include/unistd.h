#ifndef _UNISTD_H
#define _UNISTD_H

#include <stddef.h>
#include <sys/types.h>

typedef unsigned long useconds_t;

int isatty(int fd);
unsigned int sleep(unsigned int seconds);
int usleep(useconds_t useconds);

#endif
