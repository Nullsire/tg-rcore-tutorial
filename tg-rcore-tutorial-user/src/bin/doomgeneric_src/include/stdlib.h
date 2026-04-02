#ifndef _STDLIB_H
#define _STDLIB_H

#include <stddef.h>

#define NULL ((void*)0)
#define EXIT_FAILURE 1
#define EXIT_SUCCESS 0
#define RAND_MAX 32767

void *malloc(size_t size);
void free(void *ptr);
void *calloc(size_t nobj, size_t size);
void *realloc(void *ptr, size_t size);
void exit(int status);
int atexit(void (*func)(void));
int abs(int x);
long labs(long x);
int atoi(const char *s);
long atol(const char *s);
int rand(void);
void srand(unsigned int seed);
char *getenv(const char *name);
int system(const char *cmd);
void qsort(void *base, size_t nmemb, size_t size,
           int (*compar)(const void *, const void *));
void *bsearch(const void *key, const void *base,
              size_t nmemb, size_t size,
              int (*compar)(const void *, const void *));
double strtod(const char *nptr, char **endptr);
long strtol(const char *nptr, char **endptr, int base);
unsigned long strtoul(const char *nptr, char **endptr, int base);

#endif
